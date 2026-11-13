use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use secrecy::ExposeSecret as _;

use super::OpenAiModerationGuardrail;
use super::error::ModerationError;
use super::wire::{ModerationRequest, ModerationResponse};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) async fn async_make_request(
    guardrail: &OpenAiModerationGuardrail,
    input_text: &str,
) -> Result<ModerationResponse, ModerationError> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {}", guardrail.api_key.expose_secret()))
            .map_err(|source| ModerationError::InvalidHeader { source })?;
    authorization.set_sensitive(true);
    let response = guardrail
        .client
        .post(format!("{}/moderations", guardrail.api_base))
        .header(AUTHORIZATION, authorization)
        .header(CONTENT_TYPE, "application/json")
        .json(&ModerationRequest {
            model: guardrail.model,
            input: input_text,
        })
        .timeout(guardrail.timeout)
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    let body = match read_capped_body(response).await {
        // An oversized error body must not hide the upstream status.
        Err(ModerationError::ResponseTooLarge { .. }) if status != StatusCode::OK => Vec::new(),
        result => result?,
    };
    if status != StatusCode::OK {
        let text = String::from_utf8(body)
            .map_err(|source| ModerationError::ApiBodyNotUtf8 { status, source })?;
        return Err(ModerationError::Api { status, body: text });
    }
    serde_json::from_slice(&body).map_err(|source| ModerationError::ResponseJson { source })
}

async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, ModerationError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ModerationError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn request_error(source: reqwest::Error) -> ModerationError {
    if source.is_timeout() {
        ModerationError::Timeout { source }
    } else {
        ModerationError::Request { source }
    }
}
