use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use serde::Serialize;
use thiserror::Error;
use url::form_urlencoded;

use super::params::WxoRequestParams;
use super::token_cache::{TokenStore, token_cache_key};
use super::token_response::{TokenFlow, TokenGrant, TokenResponseError, decode_wxo_token_response};

mod tests;

const IBM_CLOUD_IAM_URL: &str = "https://iam.cloud.ibm.com/identity/token";
const TOKEN_CACHE_TTL_BUFFER_SECONDS: i64 = 60;

pub trait TokenClock: Send + Sync {
    fn monotonic(&self) -> Duration;
    fn unix_seconds(&self) -> i64;
}

#[derive(Debug)]
pub struct TokenRequest {
    pub url: String,
    pub content_type: &'static str,
    pub body: SecretSlice<u8>,
}

#[derive(Debug)]
pub struct TokenHttpResponse {
    pub status: u16,
    pub body: SecretSlice<u8>,
}

pub trait TokenTransport: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn post(
        &self,
        request: &TokenRequest,
    ) -> impl Future<Output = Result<TokenHttpResponse, Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum CredentialsError<E> {
    #[error("auth_mode 'cp4d' needs a 'username' in litellm_params")]
    MissingUsername,
    #[error("token request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("token request failed")]
    Transport(#[source] E),
    #[error("token endpoint answered with HTTP status {status}")]
    Status { status: u16 },
    #[error("token endpoint answer is unusable")]
    Decode(#[source] TokenResponseError),
}

pub struct BearerTokens<T, C, S> {
    pub transport: T,
    pub clock: C,
    pub store: S,
}

#[derive(Serialize)]
struct Cp4dAuthorize<'secret> {
    username: &'secret str,
    api_key: &'secret str,
}

fn build_request<E>(
    params: &WxoRequestParams,
    flow: TokenFlow,
) -> Result<TokenRequest, CredentialsError<E>> {
    match flow {
        TokenFlow::IbmCloud => {
            let form = form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "urn:ibm:params:oauth:grant-type:apikey")
                .append_pair("apikey", params.api_key.expose_secret())
                .finish();
            Ok(TokenRequest {
                url: IBM_CLOUD_IAM_URL.to_owned(),
                content_type: "application/x-www-form-urlencoded",
                body: SecretSlice::from(form.into_bytes()),
            })
        }
        TokenFlow::Cp4d => {
            let username = params
                .username
                .as_deref()
                .ok_or(CredentialsError::MissingUsername)?;
            let body = serde_json::to_vec(&Cp4dAuthorize {
                username,
                api_key: params.api_key.expose_secret(),
            })
            .map_err(CredentialsError::Encode)?;
            Ok(TokenRequest {
                url: format!(
                    "{}/icp4d-api/v1/authorize",
                    params.cp4d_host.trim_end_matches('/')
                ),
                content_type: "application/json",
                body: SecretSlice::from(body),
            })
        }
    }
}

impl<T: TokenTransport, C: TokenClock, S: TokenStore> BearerTokens<T, C, S> {
    pub async fn bearer_token(
        &self,
        params: &WxoRequestParams,
    ) -> Result<SecretString, CredentialsError<T::Error>> {
        let now = self.clock.monotonic();
        self.store.evict_expired(now);
        let key = token_cache_key(
            &params.auth_mode,
            &params.cp4d_host,
            params.username.as_deref(),
            &params.api_key,
        );
        if let Some(token) = self.store.lookup(&key, now) {
            return Ok(token);
        }
        let flow = if params.auth_mode == "ibm_cloud" {
            TokenFlow::IbmCloud
        } else {
            TokenFlow::Cp4d
        };
        let request = build_request(params, flow)?;
        let response = self
            .transport
            .post(&request)
            .await
            .map_err(CredentialsError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(CredentialsError::Status {
                status: response.status,
            });
        }
        let grant = decode_wxo_token_response(
            response.body.expose_secret(),
            flow,
            self.clock.unix_seconds(),
        )
        .map_err(CredentialsError::Decode)?;
        self.remember(key, now, &grant);
        Ok(grant.token)
    }

    fn remember(&self, key: String, now: Duration, grant: &TokenGrant) {
        let cached_seconds = grant
            .lifetime_seconds
            .saturating_sub(TOKEN_CACHE_TTL_BUFFER_SECONDS);
        // a token with no life left after the buffer is returned but never cached
        if let Ok(seconds @ 1..) = u64::try_from(cached_seconds) {
            let expires_at = now.saturating_add(Duration::from_secs(seconds));
            self.store.insert(
                key,
                SecretString::from(grant.token.expose_secret()),
                expires_at,
            );
        }
    }
}
