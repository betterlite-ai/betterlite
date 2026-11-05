use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, InvalidHeaderValue};
use http::{Method, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use crate::arize::phoenix::records::{PromptVersionData, PromptVersionEnvelope};
use crate::arize::phoenix::sanitization::{SanitizationError, sanitize_id};

// LiteLLM's HTTPHandler waits as long as its default request timeout, 6000 seconds.
const REQUEST_DEADLINE: Duration = Duration::from_mins(100);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Sanitization(#[from] SanitizationError),
    #[error(
        "Access denied to prompt version '{prompt_version_id}'. Check your Arize Phoenix permissions."
    )]
    AccessDenied { prompt_version_id: String },
    #[error("Authentication failed. Check your Arize Phoenix API key and permissions.")]
    AuthenticationFailed,
    #[error("Failed to fetch prompt version '{prompt_version_id}': {detail}")]
    Status {
        prompt_version_id: String,
        detail: String,
    },
    #[error("Error fetching prompt version '{prompt_version_id}': {source}")]
    InvalidUrl {
        prompt_version_id: String,
        #[source]
        source: url::ParseError,
    },
    #[error("Error fetching prompt version '{prompt_version_id}': {source}")]
    InvalidApiKey {
        prompt_version_id: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("Error fetching prompt version '{prompt_version_id}': {source}")]
    Transport {
        prompt_version_id: String,
        #[source]
        source: TransportError,
    },
    #[error("Error fetching prompt version '{prompt_version_id}': {source}")]
    Decode {
        prompt_version_id: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug)]
pub struct PhoenixClient {
    pub http: ProxiedClient,
    pub api_key: SecretString,
    pub api_base: String,
}

impl PhoenixClient {
    fn build_request(&self, prompt_version_id: &str) -> Result<OutboundRequest, ClientError> {
        let safe_id = sanitize_id(prompt_version_id)?;
        let url = Url::parse(&format!("{}/v1/prompt_versions/{safe_id}", self.api_base)).map_err(
            |source| ClientError::InvalidUrl {
                prompt_version_id: prompt_version_id.to_owned(),
                source,
            },
        )?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose_secret())).map_err(
                |source| ClientError::InvalidApiKey {
                    prompt_version_id: prompt_version_id.to_owned(),
                    source,
                },
            )?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        Ok(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: REQUEST_DEADLINE,
        })
    }

    // None means Phoenix answered 404 for this version.
    pub async fn get_prompt_version(
        &self,
        prompt_version_id: &str,
    ) -> Result<Option<PromptVersionData>, ClientError> {
        let request = self.build_request(prompt_version_id)?;
        let url = request.url.clone();
        let response =
            self.http
                .send_buffered(request)
                .await
                .map_err(|source| ClientError::Transport {
                    prompt_version_id: prompt_version_id.to_owned(),
                    source,
                })?;
        match response.status {
            200..=299 => {
                let envelope: PromptVersionEnvelope = serde_json::from_slice(&response.body)
                    .map_err(|source| ClientError::Decode {
                        prompt_version_id: prompt_version_id.to_owned(),
                        source,
                    })?;
                Ok(envelope.data)
            }
            404 => Ok(None),
            403 => Err(ClientError::AccessDenied {
                prompt_version_id: prompt_version_id.to_owned(),
            }),
            401 => Err(ClientError::AuthenticationFailed),
            status => Err(ClientError::Status {
                prompt_version_id: prompt_version_id.to_owned(),
                detail: status_detail(status, &url),
            }),
        }
    }
}

// The text httpx's raise_for_status puts on a non-success response.
fn status_detail(status: u16, url: &Url) -> String {
    let kind = match status {
        100..=199 => "Informational response",
        300..=399 => "Redirect response",
        400..=499 => "Client error",
        _ => "Server error",
    };
    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or_default();
    format!(
        "{kind} '{status} {reason}' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/{status}"
    )
}

mod tests;
