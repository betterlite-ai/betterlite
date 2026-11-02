use std::ops::Sub as _;
use std::str;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::json::Json;
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::error::ApiError;

const CISCO_API_KEY_HEADER: &str = "x-cisco-ai-defense-api-key";
const LITELLM_VERSION: &str = "1.105.1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

// Mirrors LiteLLM's fn _build_headers.
pub fn build_headers(guardrail: &CiscoInspectionGuardrail) -> Result<HeaderMap, ApiError> {
    let mut headers = HeaderMap::new();
    let mut api_key = HeaderValue::from_str(guardrail.settings.api_key.expose_secret())
        .map_err(|source| ApiError::InvalidApiKeyHeader { source })?;
    api_key.set_sensitive(true);
    headers.insert(HeaderName::from_static(CISCO_API_KEY_HEADER), api_key);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let user_agent = HeaderValue::from_str(&format!("litellm/{LITELLM_VERSION}"))
        .map_err(|source| ApiError::InvalidUserAgentHeader { source })?;
    headers.insert(USER_AGENT, user_agent);
    Ok(headers)
}

// Mirrors LiteLLM's fn _post_inspection; see docs/deviations/B3762.yaml for the redirect policy.
pub async fn post_inspection<Payload>(
    guardrail: &CiscoInspectionGuardrail,
    url: &str,
    payload: &Payload,
    surface: &str,
) -> Result<Json, ApiError>
where
    Payload: Serialize + Sync,
{
    let headers = build_headers(guardrail)?;
    let timeout =
        Duration::try_from_secs_f64(guardrail.settings.timeout).unwrap_or(DEFAULT_TIMEOUT);
    let response = guardrail
        .client
        .post(url)
        .headers(headers)
        .json(payload)
        .timeout(timeout)
        .send()
        .await
        .map_err(|error| send_error(error, surface, guardrail.settings.timeout))?;
    let status = response.status();
    let body = match read_body(response, surface, guardrail.settings.timeout).await {
        // Keep the HTTP status authoritative when an error body exceeds the limit.
        Err(ApiError::ResponseTooLarge { .. }) if !status.is_success() => Vec::new(),
        result => result?,
    };
    if !status.is_success() {
        let text = str::from_utf8(&body).map_err(|source| ApiError::HttpStatusEncoding {
            surface: surface.to_owned(),
            status: status.as_u16(),
            source,
        })?;
        let snippet: String = text.chars().take(500).collect();
        return Err(ApiError::HttpStatus {
            surface: surface.to_owned(),
            status: status.as_u16(),
            snippet,
        });
    }
    parse_body(&body, surface)
}

async fn read_body(
    mut response: reqwest::Response,
    surface: &str,
    timeout: f64,
) -> Result<Vec<u8>, ApiError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| send_error(error, surface, timeout))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ApiError::ResponseTooLarge {
                surface: surface.to_owned(),
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_body(body: &[u8], surface: &str) -> Result<Json, ApiError> {
    serde_json::from_slice::<Box<RawValue>>(body)
        .map(|raw| Json::parse(raw.get()))
        .map_err(|source| ApiError::ResponseJson {
            surface: surface.to_owned(),
            source,
        })
}

fn send_error(source: reqwest::Error, surface: &str, timeout: f64) -> ApiError {
    if source.is_timeout() {
        ApiError::Timeout {
            surface: surface.to_owned(),
            timeout: render_timeout_seconds(timeout),
            source,
        }
    } else {
        ApiError::Request {
            surface: surface.to_owned(),
            source,
        }
    }
}

// Matches LiteLLM's f-string rendering of a whole-number timeout with a trailing .0.
fn render_timeout_seconds(value: f64) -> String {
    if value.sub(value.trunc()).abs() < f64::EPSILON {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

mod tests;
