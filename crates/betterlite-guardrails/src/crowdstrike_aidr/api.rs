use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use crate::crowdstrike_aidr::error::{CrowdStrikeError, MALFORMED_TRANSFORMED_MESSAGE};
use crate::crowdstrike_aidr::handler::CrowdStrikeAidr;
use crate::crowdstrike_aidr::models::{GuardPayload, GuardResponse, GuardResult, GuardSignals};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const GUARD_PATH: &str = "/v1/guard_chat_completions";

// LiteLLM's _call_crowdstrike_aidr_guard
pub async fn call_guard(
    guardrail: &CrowdStrikeAidr,
    payload: &GuardPayload,
    hook_name: &str,
) -> Result<GuardResult, CrowdStrikeError> {
    let endpoint = format!("{}{GUARD_PATH}", guardrail.settings.api_base);
    tracing::debug!("CrowdStrike AIDR Guardrail ({hook_name}): Calling endpoint {endpoint}");
    let timeout = guardrail
        .settings
        .timeout
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .unwrap_or(DEFAULT_TIMEOUT);
    let response = guardrail
        .client
        .post(&endpoint)
        .headers(auth_headers(guardrail)?)
        .json(payload)
        .timeout(timeout)
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(CrowdStrikeError::HttpStatus { status });
    }
    let body = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
    parse_guard_result(&body, guardrail.settings.guardrail_name.as_deref())
}

fn auth_headers(guardrail: &CrowdStrikeAidr) -> Result<HeaderMap, CrowdStrikeError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let bearer = format!("Bearer {}", guardrail.settings.api_key.expose_secret());
    let mut authorization = HeaderValue::from_str(&bearer)
        .map_err(|source| CrowdStrikeError::InvalidHeader { source })?;
    authorization.set_sensitive(true);
    headers.insert(HeaderName::from_static("authorization"), authorization);
    Ok(headers)
}

// A blocked verdict is read before the body is validated, so it survives a malformed guard_output
fn parse_guard_result(
    body: &[u8],
    guardrail_name: Option<&str>,
) -> Result<GuardResult, CrowdStrikeError> {
    let signals: GuardSignals = serde_json::from_slice(body).unwrap_or_default();
    let raw_transformed = signals
        .result
        .as_ref()
        .and_then(|result| result.transformed);
    if let Some(blocked) = signals.result.as_ref().and_then(|result| result.blocked)
        && blocked
    {
        return Err(CrowdStrikeError::Blocked {
            guardrail_name: guardrail_name.map(str::to_owned),
        });
    }
    match serde_json::from_slice::<GuardResponse>(body) {
        Ok(response) => Ok(response.result.unwrap_or_default()),
        Err(source) => {
            if raw_transformed == Some(true) {
                tracing::error!("{MALFORMED_TRANSFORMED_MESSAGE}");
                return Err(CrowdStrikeError::MalformedTransformed { source });
            }
            Err(CrowdStrikeError::ResponseJson { source })
        }
    }
}

fn request_error(source: reqwest::Error) -> CrowdStrikeError {
    if source.is_timeout() {
        CrowdStrikeError::Timeout { source }
    } else {
        CrowdStrikeError::Request { source }
    }
}

async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, CrowdStrikeError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(CrowdStrikeError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(CrowdStrikeError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
