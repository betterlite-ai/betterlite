use std::io::{self, Write};
use std::sync::Mutex;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use uuid::Uuid;

use super::error::XaiLoginError;
use super::listener::{CallbackServer, CallbackSettings};
use super::pkce::{Entropy, build_authorize_url, pkce_pair};
use crate::xai_tokens::Clock;
use crate::xai_tokens::XaiOauth;
use crate::xai_tokens::codec::{AuthFile, Numeric, decode_xai_oauth};
use crate::xai_tokens::error::XaiOauthError;
use crate::xai_tokens::files::FileAccess;
use crate::xai_tokens::rules::{
    CLIENT_ID, OauthRecord, build_auth_record, is_expired, validate_xai_endpoint,
};
use crate::xai_tokens::transport::Transport;

pub trait Uuids: Send + Sync {
    fn next(&self) -> String;
}

pub struct RandomUuids;

impl Uuids for RandomUuids {
    fn next(&self) -> String {
        Uuid::new_v4().simple().to_string()
    }
}

pub trait Browser: Send + Sync {
    fn open(&self, url: &str) -> bool;
}

pub trait UserOutput: Send + Sync {
    fn write(&self, text: &str) -> io::Result<()>;
}

pub struct TerminalOutput<W> {
    pub writer: Mutex<W>,
}

impl<W: Write + Send> UserOutput for TerminalOutput<W> {
    fn write(&self, text: &str) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_error| io::Error::other("the terminal writer lock is poisoned"))?;
        writer.write_all(text.as_bytes())?;
        writer.flush()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoginOptions {
    pub force: bool,
    pub no_browser: bool,
}

pub struct XaiLogin<F, C, T, E, U, B, O> {
    pub oauth: XaiOauth<F, C, T>,
    pub entropy: E,
    pub uuids: U,
    pub browser: B,
    pub output: O,
    pub callbacks: CallbackSettings,
}

impl<F: FileAccess, C: Clock, T: Transport, E: Entropy, U: Uuids, B: Browser, O: UserOutput>
    XaiLogin<F, C, T, E, U, B, O>
{
    pub async fn login(&self, options: LoginOptions) -> Result<OauthRecord, XaiLoginError> {
        let now = self.oauth.clock.now();
        let existing = self
            .read_stored()
            .await
            .filter(|_| !options.force)
            .map(|stored| classify(stored, now));
        match existing {
            Some(Existing::Fresh(record)) => Ok(record),
            Some(Existing::Refreshable(auth)) => match self.oauth.refresh_tokens(&auth).await {
                Ok(record) => Ok(record),
                Err(error) if falls_through(&error) => self.interactive(options).await,
                Err(error) => Err(XaiLoginError::Oauth(error)),
            },
            Some(Existing::Unusable) | None => self.interactive(options).await,
        }
    }

    async fn interactive(&self, options: LoginOptions) -> Result<OauthRecord, XaiLoginError> {
        let (authorization_endpoint, token_endpoint) = self.oauth.discover().await?;
        let (verifier, challenge) = pkce_pair(&self.entropy)?;
        let state = self.uuids.next();
        let nonce = self.uuids.next();
        let server = CallbackServer::bind(&self.callbacks, &state).await?;
        let redirect_uri = server.redirect_uri.clone();
        let authorize_url = build_authorize_url(
            &authorization_endpoint,
            &redirect_uri,
            &challenge,
            &state,
            &nonce,
        );
        if options.no_browser || !self.browser.open(&authorize_url) {
            self.output
                .write(&format!(
                    "Open this URL to authenticate with xAI:\n{authorize_url}\n"
                ))
                .map_err(|source| XaiLoginError::Output { source })?;
        }

        let request = server.wait(&self.oauth.clock).await?;
        if request.state.as_deref() != Some(state.as_str()) {
            return Err(XaiLoginError::StateMismatch);
        }
        if let Some(error) = request.error {
            let description = request.error_description.unwrap_or(error);
            return Err(XaiLoginError::Provider { description });
        }
        let Some(code) = request.code else {
            return Err(XaiLoginError::MissingCode);
        };
        self.redeem(&code, &verifier, &token_endpoint, &redirect_uri)
            .await
    }

    async fn redeem(
        &self,
        code: &SecretString,
        verifier: &SecretString,
        token_endpoint: &str,
        redirect_uri: &str,
    ) -> Result<OauthRecord, XaiLoginError> {
        let endpoint = validate_xai_endpoint(token_endpoint)?;
        let payload = self
            .oauth
            .exchange_token(
                endpoint,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code.expose_secret()),
                    ("redirect_uri", redirect_uri),
                    ("client_id", CLIENT_ID),
                    ("code_verifier", verifier.expose_secret()),
                ],
            )
            .await?;
        let record = build_auth_record(self.oauth.clock.now(), payload, token_endpoint, None)?;
        self.oauth.write_auth_file(&record).await?;
        Ok(record)
    }

    async fn read_stored(&self) -> Option<StoredRecord> {
        let bytes = self.oauth.files.read(&self.oauth.auth_file).await.ok()?;
        decode_xai_oauth(&bytes)
            .map_err(|error| tracing::debug!(%error, "Ignoring an unreadable xAI OAuth auth file"))
            .ok()
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct StoredRecord {
    access_token: Option<SecretString>,
    refresh_token: Option<SecretString>,
    id_token: Option<SecretString>,
    token_type: String,
    token_endpoint: String,
    expires_at: Option<Numeric>,
}

pub(super) enum Existing {
    Fresh(OauthRecord),
    Refreshable(AuthFile),
    Unusable,
}

pub(super) fn classify(stored: StoredRecord, now: Decimal) -> Existing {
    let Some(access_token) = stored
        .access_token
        .filter(|token| !token.expose_secret().is_empty())
    else {
        return Existing::Unusable;
    };
    let refresh_token = stored
        .refresh_token
        .unwrap_or_else(|| SecretString::from(String::new()));
    let refreshable = !refresh_token.expose_secret().is_empty();
    let expires_at = stored
        .expires_at
        .as_ref()
        .and_then(Numeric::decimal)
        .and_then(|value| value.trunc().to_i64());
    match expires_at {
        Some(seconds) if !is_expired(now, stored.expires_at.as_ref()) => {
            Existing::Fresh(OauthRecord {
                access_token,
                refresh_token,
                id_token: stored.id_token,
                token_type: stored.token_type,
                token_endpoint: stored.token_endpoint,
                expires_at: seconds,
            })
        }
        _ if refreshable => Existing::Refreshable(AuthFile {
            access_token: Some(access_token),
            refresh_token: Some(refresh_token),
            token_endpoint: Some(stored.token_endpoint),
            expires_at: stored.expires_at,
        }),
        _ => Existing::Unusable,
    }
}

const fn falls_through(error: &XaiOauthError) -> bool {
    matches!(
        error,
        XaiOauthError::LoginRequired
            | XaiOauthError::RefreshTokenMissing
            | XaiOauthError::Status { .. }
            | XaiOauthError::Response { .. }
            | XaiOauthError::MissingEndpoints
            | XaiOauthError::UnexpectedEndpoint
            | XaiOauthError::MissingAccessToken
            | XaiOauthError::MissingRefreshToken
    )
}
