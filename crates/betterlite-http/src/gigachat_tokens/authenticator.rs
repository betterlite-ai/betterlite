use std::str;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::gigachat_auth::Settings;
use http::StatusCode;
use http::header::AUTHORIZATION;
use reqwest::Response;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::gigachat_tokens::cache::{EXPIRY_BUFFER_MILLIS, TokenCache};
use crate::gigachat_tokens::error::GigachatAuthError;
use crate::gigachat_tokens::reply::{MAX_REPLY_BYTES, TokenReply, decode_token_reply};
use crate::gigachat_tokens::request::GigachatTokenRequest;
use crate::gigachat_tokens::runtime::GigachatRuntime;
use crate::gigachat_tokens::tokens::GigachatTokens;

const OAUTH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REJECTED_BYTES: usize = 1024;

pub struct GigachatAuthenticator<R: GigachatRuntime> {
    settings: Settings,
    client: reqwest::Client,
    runtime: R,
    cache: TokenCache,
}

impl<R: GigachatRuntime> GigachatAuthenticator<R> {
    pub fn new(settings: Settings, client: reqwest::Client, runtime: R) -> Self {
        Self {
            settings,
            client,
            runtime,
            cache: TokenCache::default(),
        }
    }

    fn direct_token(&self, request: &GigachatTokenRequest) -> Option<SecretString> {
        request
            .access_token
            .as_ref()
            .filter(|token| !token.expose_secret().is_empty())
            .or_else(|| {
                self.settings
                    .access_token
                    .as_ref()
                    .filter(|token| !token.expose_secret().is_empty())
            })
            .cloned()
    }

    fn credentials<'request>(
        &'request self,
        request: &'request GigachatTokenRequest,
    ) -> Result<&'request SecretString, GigachatAuthError> {
        request
            .credentials
            .as_ref()
            .filter(|credentials| !credentials.expose_secret().is_empty())
            .or_else(|| {
                self.settings
                    .credentials
                    .as_ref()
                    .filter(|credentials| !credentials.expose_secret().is_empty())
            })
            .ok_or(GigachatAuthError::MissingCredentials)
    }

    async fn request_token(
        &self,
        credentials: &SecretString,
        scope: &str,
        auth_url: &str,
    ) -> Result<TokenReply, GigachatAuthError> {
        tracing::debug!("Requesting GigaChat access token from {auth_url}");
        let request = oauth_request(
            &self.client,
            auth_url,
            credentials,
            self.runtime.request_uid(),
            scope,
        )?;
        let response = self
            .client
            .execute(request)
            .await
            .map_err(|source| GigachatAuthError::Transport { source })?;
        let status = response.status();
        if !status.is_success() {
            let body = read_prefix(response, MAX_REJECTED_BYTES)
                .await
                .map_err(|source| GigachatAuthError::Transport { source })?;
            return Err(rejected(status, &body));
        }
        let body = read_prefix(response, MAX_REPLY_BYTES.saturating_add(1))
            .await
            .map_err(|source| GigachatAuthError::Transport { source })?;
        if body.len() > MAX_REPLY_BYTES {
            return Err(GigachatAuthError::InvalidReply);
        }
        let reply = decode_token_reply(&body)?;
        tracing::debug!("GigaChat access token obtained successfully");
        Ok(reply)
    }

    async fn remember(&self, cache_key: &str, reply: &TokenReply) {
        let Some(expires_at) = reply.expires_at else {
            return;
        };
        if expires_at == 0 {
            return;
        }
        let now_millis = self.runtime.now_millis();
        if now_millis < expires_at.saturating_sub(EXPIRY_BUFFER_MILLIS) {
            self.cache
                .insert(
                    cache_key.to_owned(),
                    reply.token.clone(),
                    expires_at,
                    now_millis,
                )
                .await;
        }
    }
}

#[async_trait]
impl<R: GigachatRuntime> GigachatTokens for GigachatAuthenticator<R> {
    async fn access_token(
        &self,
        request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError> {
        if let Some(token) = self.direct_token(request) {
            return Ok(token);
        }
        let credentials = self.credentials(request)?;
        let scope = request
            .scope
            .as_deref()
            .filter(|scope| !scope.is_empty())
            .unwrap_or(&self.settings.scope);
        let auth_url = request
            .auth_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .unwrap_or(&self.settings.auth_url);
        let prefix: String = credentials.expose_secret().chars().take(16).collect();
        let cache_key = format!("gigachat_token:{prefix}");
        let now_millis = self.runtime.now_millis();
        if let Some(token) = self.cache.lookup(&cache_key, now_millis).await {
            tracing::debug!("Using cached GigaChat access token");
            return Ok(token);
        }
        let reply = self.request_token(credentials, scope, auth_url).await?;
        self.remember(&cache_key, &reply).await;
        Ok(reply.token)
    }
}

pub(super) fn oauth_request(
    client: &reqwest::Client,
    auth_url: &str,
    credentials: &SecretString,
    request_uid: Uuid,
    scope: &str,
) -> Result<reqwest::Request, GigachatAuthError> {
    let mut request = client
        .post(auth_url)
        .header(
            AUTHORIZATION,
            format!("Basic {}", credentials.expose_secret()),
        )
        .header("RqUID", request_uid.to_string())
        .form(&[("scope", scope)])
        .timeout(OAUTH_TIMEOUT)
        .build()
        .map_err(|source| GigachatAuthError::Transport { source })?;
    // A basic credential never reaches Debug output or a log line
    if let Some(value) = request.headers_mut().get_mut(AUTHORIZATION) {
        value.set_sensitive(true);
    }
    Ok(request)
}

async fn read_prefix(mut response: Response, limit: usize) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    loop {
        let remaining = limit.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        let keep = remaining.min(chunk.len());
        body.extend_from_slice(chunk.get(..keep).unwrap_or_default());
    }
    Ok(body)
}

fn rejected(status: StatusCode, body: &[u8]) -> GigachatAuthError {
    // A reply that is not UTF-8 keeps the text up to its first invalid byte
    let text = match str::from_utf8(body) {
        Ok(text) => text,
        Err(invalid) => str::from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default(),
    };
    GigachatAuthError::Rejected {
        status,
        body: text.to_owned(),
    }
}
