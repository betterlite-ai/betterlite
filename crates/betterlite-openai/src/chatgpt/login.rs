use std::collections::BTreeMap;
use std::time::Duration;

use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use self::codec::{
    AuthorizationCode, AuthorizationReply, DecodeError, DeviceCode, DeviceReply, TokenReply,
    Tokens, decode_chatgpt_oauth,
};
use self::error::{Failure, LoginError};
use self::http::{AuthPost, AuthTransport, Body};
use super::token_store::codec::{AuthRecord, build_auth_record};
use super::token_store::files::FileAccess;
use super::token_store::member::Member;
use super::token_store::{ChatGptTokenStore, Clock};

pub mod codec;
pub mod error;
pub mod http;
mod tests;

pub const DEVICE_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const POLL_TIMEOUT: Decimal = Decimal::from_parts(900, 0, 0, false, 0);
const COOLDOWN: Duration = Duration::from_mins(5);
const NANOSECONDS: Decimal = Decimal::from_parts(1_000_000_000, 0, 0, false, 0);

pub trait Pause: Clock {
    fn pause(&mut self, duration: Duration) -> impl Future<Output = ()> + Send;
}

pub trait DeviceDisplay {
    fn show(&mut self, verify_url: &str, user_code: &SecretString);
}

pub struct ChatGptLogin<F, C, H, D> {
    pub store: ChatGptTokenStore<F, C>,
    pub http: H,
    pub display: D,
}

#[derive(Serialize)]
struct DeviceCodeBody {
    client_id: &'static str,
}

#[derive(Serialize)]
struct PollBody<'code> {
    device_auth_id: &'code str,
    user_code: &'code str,
}

#[derive(Serialize)]
struct RefreshBody<'code> {
    client_id: &'static str,
    grant_type: &'static str,
    refresh_token: &'code str,
    scope: &'static str,
}

fn usable(member: Option<&Member<SecretString>>) -> Option<SecretString> {
    member
        .and_then(Member::known)
        .filter(|token| !token.expose_secret().is_empty())
        .cloned()
}

fn stored_record(tokens: &Tokens) -> AuthRecord {
    build_auth_record(AuthRecord {
        access_token: Some(Member::Known(tokens.access_token.clone())),
        refresh_token: Some(Member::Known(tokens.refresh_token.clone())),
        id_token: Some(Member::Known(tokens.id_token.clone())),
        expires_at: None,
        account_id: None,
        device_code_requested_at: None,
        additional_fields: BTreeMap::new(),
    })
}

impl<F, C, H, D> ChatGptLogin<F, C, H, D>
where
    F: FileAccess + Send,
    C: Pause + Send,
    H: AuthTransport + Send,
    D: DeviceDisplay + Send,
{
    pub async fn get_access_token(&mut self) -> Result<SecretString, LoginError> {
        let mut record = self.store.read();
        if let Some(current) = record.as_mut() {
            if let Some(token) = usable(current.access_token.as_ref())
                && !self.store.is_expired(current, &token)
            {
                return Ok(token);
            }
            if let Some(token) = self.refreshed_token(current).await {
                return Ok(token);
            }
        }
        if let Some(token) = self.wait_out_cooldown(record.as_ref()) {
            return Ok(token);
        }
        self.login_device_code().await
    }

    async fn refreshed_token(&mut self, record: &AuthRecord) -> Option<SecretString> {
        let refresh_token = usable(record.refresh_token.as_ref())?;
        match self.refresh_tokens(&refresh_token).await {
            Ok(access_token) => Some(access_token),
            Err(error) => {
                tracing::warn!(
                    status = error.status(),
                    "ChatGPT refresh token failed, re-login required"
                );
                None
            }
        }
    }

    fn wait_out_cooldown(&mut self, record: Option<&AuthRecord>) -> Option<SecretString> {
        let remaining = self.store.cooldown_remaining(record);
        if remaining <= Decimal::ZERO {
            return None;
        }
        // a record dated far in the future must not hold a login longer than the cooldown itself
        let wait = remaining
            .checked_mul(NANOSECONDS)
            .and_then(|nanos| nanos.to_u64())
            .map_or(COOLDOWN, Duration::from_nanos)
            .min(COOLDOWN);
        self.store.wait_for_access_token(wait)
    }

    pub async fn login_device_code(&mut self) -> Result<SecretString, LoginError> {
        let record = self.store.read();
        if let Some(token) = self.wait_out_cooldown(record.as_ref()) {
            return Ok(token);
        }
        let device = self.request_device_code().await?;
        self.store.record_device_request();
        self.display.show(DEVICE_VERIFY_URL, &device.user_code);
        let authorization = self.poll_for_authorization_code(&device).await?;
        let tokens = self.exchange_code_for_tokens(&authorization).await?;
        self.store.write(&stored_record(&tokens));
        Ok(tokens.access_token)
    }

    async fn request_device_code(&mut self) -> Result<DeviceCode, LoginError> {
        let attempt = async {
            let body = Body::Json(SecretString::from(serde_json::to_string(
                &DeviceCodeBody {
                    client_id: CLIENT_ID,
                },
            )?));
            let reply = self
                .http
                .post(AuthPost {
                    url: DEVICE_CODE_URL,
                    body,
                })
                .await?
                .into_body()?;
            Ok::<_, Failure>(decode_chatgpt_oauth::<DeviceReply>(&reply)?.into_code()?)
        };
        attempt.await.map_err(LoginError::DeviceCode)
    }

    async fn poll_for_authorization_code(
        &mut self,
        device: &DeviceCode,
    ) -> Result<AuthorizationCode, LoginError> {
        let started = self.store.clock.now();
        while self
            .store
            .clock
            .now()
            .checked_sub(started)
            .is_some_and(|elapsed| elapsed < POLL_TIMEOUT)
        {
            match self.poll_once(device).await {
                Ok(Some(code)) => return Ok(code),
                Ok(None) => self.store.clock.pause(device.interval).await,
                Err(failure) => return Err(LoginError::AccessToken(failure)),
            }
        }
        Err(LoginError::AccessToken(Failure::Expired))
    }

    async fn poll_once(
        &mut self,
        device: &DeviceCode,
    ) -> Result<Option<AuthorizationCode>, Failure> {
        let body = Body::Json(SecretString::from(serde_json::to_string(&PollBody {
            device_auth_id: device.device_auth_id.expose_secret(),
            user_code: device.user_code.expose_secret(),
        })?));
        let response = self
            .http
            .post(AuthPost {
                url: DEVICE_TOKEN_URL,
                body,
            })
            .await?;
        let status = response.status;
        if status == 200 {
            let code = decode_chatgpt_oauth::<AuthorizationReply>(&response.body)?.complete();
            if code.is_some() {
                return Ok(code);
            }
        }
        // 403 and 404 mean the user has not approved yet
        if matches!(status, 403 | 404) || (200..300).contains(&status) {
            Ok(None)
        } else {
            Err(Failure::Status(status))
        }
    }

    async fn token_call<Finish>(&mut self, body: Body, finish: Finish) -> Result<Tokens, Failure>
    where
        Finish: FnOnce(TokenReply) -> Result<Tokens, DecodeError>,
    {
        let reply = self
            .http
            .post(AuthPost {
                url: OAUTH_TOKEN_URL,
                body,
            })
            .await?
            .into_body()?;
        Ok(finish(decode_chatgpt_oauth::<TokenReply>(&reply)?)?)
    }

    async fn exchange_code_for_tokens(
        &mut self,
        code: &AuthorizationCode,
    ) -> Result<Tokens, LoginError> {
        // the source concatenates these values without form escaping
        let body = Body::Form(SecretString::from(format!(
            "grant_type=authorization_code&code={}&redirect_uri={REDIRECT_URI}&client_id={CLIENT_ID}&code_verifier={}",
            code.authorization_code.expose_secret(),
            code.code_verifier.expose_secret(),
        )));
        self.token_call(body, TokenReply::exchanged)
            .await
            .map_err(LoginError::AccessToken)
    }

    pub async fn refresh_tokens(
        &mut self,
        refresh_token: &SecretString,
    ) -> Result<SecretString, LoginError> {
        let attempt = async {
            let body = Body::Json(SecretString::from(serde_json::to_string(&RefreshBody {
                client_id: CLIENT_ID,
                grant_type: "refresh_token",
                refresh_token: refresh_token.expose_secret(),
                scope: "openid profile email",
            })?));
            self.token_call(body, |reply| reply.refreshed(refresh_token.clone()))
                .await
        };
        let refreshed = attempt.await.map_err(LoginError::Refresh)?;
        self.store.write(&stored_record(&refreshed));
        Ok(refreshed.access_token)
    }
}
