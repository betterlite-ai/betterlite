use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::form_urlencoded;

use crate::datadog_logging::clock::Clock;
use crate::datadog_logging::transport::{LOGGING_TIMEOUT, PostRequest, Transport};

use super::error::SentinelError;
use super::settings::SentinelSettings;

// a token is refreshed this long before it expires
const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const DEFAULT_EXPIRES_IN_SECONDS: f64 = 3600.0;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<SecretString>,
    expires_in: Option<f64>,
}

#[derive(Debug)]
struct CachedToken {
    value: SecretString,
    expires_at: Duration,
}

#[derive(Debug, Default)]
pub struct TokenCache {
    cached: Mutex<Option<CachedToken>>,
}

fn token_request(settings: &SentinelSettings) -> PostRequest {
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &settings.client_id)
        .append_pair("client_secret", settings.client_secret.expose_secret())
        .append_pair("scope", settings.scope)
        .append_pair("grant_type", "client_credentials")
        .finish();
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    PostRequest {
        url: settings.token_url.clone(),
        headers,
        body: form.into_bytes(),
        timeout: LOGGING_TIMEOUT,
    }
}

impl TokenCache {
    pub async fn bearer(
        &self,
        settings: &SentinelSettings,
        transport: &dyn Transport,
        clock: &dyn Clock,
    ) -> Result<SecretString, SentinelError> {
        let now = clock.now_unix();
        if let Some(token) = self
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|token| now < token.expires_at.saturating_sub(REFRESH_MARGIN))
        {
            return Ok(token.value.clone());
        }
        let response = transport.post(token_request(settings)).await?;
        if response.status != 200 {
            return Err(SentinelError::TokenStatus {
                status: response.status,
                detail: str::from_utf8(&response.body)
                    .unwrap_or("<non-UTF-8 body>")
                    .to_owned(),
            });
        }
        let parsed: TokenResponse =
            serde_json::from_slice(&response.body).map_err(SentinelError::TokenBody)?;
        let value = parsed
            .access_token
            .filter(|token| !token.expose_secret().is_empty())
            .ok_or(SentinelError::MissingAccessToken)?;
        let seconds = parsed.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECONDS);
        let lifetime = Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or(Duration::MAX);
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(CachedToken {
            value: value.clone(),
            expires_at: clock.now_unix().saturating_add(lifetime),
        });
        Ok(value)
    }
}
