use super::GuardrailsAiPreCallApiResponse;
use super::error::GuardrailsAiGuardrailError;

pub(super) fn request_error(source: reqwest::Error) -> GuardrailsAiGuardrailError {
    if source.is_timeout() {
        GuardrailsAiGuardrailError::Timeout { source }
    } else {
        GuardrailsAiGuardrailError::Request { source }
    }
}

pub(super) async fn read_capped_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GuardrailsAiGuardrailError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(GuardrailsAiGuardrailError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(GuardrailsAiGuardrailError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}

pub(super) fn parse_pre_call_response(
    status: reqwest::StatusCode,
    bytes: &[u8],
) -> Result<String, GuardrailsAiGuardrailError> {
    if status == reqwest::StatusCode::BAD_REQUEST {
        let body_text = String::from_utf8(bytes.to_vec())
            .map_err(|source| GuardrailsAiGuardrailError::InvalidUtf8 { source })?;
        return Err(GuardrailsAiGuardrailError::PreCallBlocked {
            response: body_text,
        });
    }
    if !status.is_success() {
        return Err(GuardrailsAiGuardrailError::HttpStatus { status });
    }
    let parsed: GuardrailsAiPreCallApiResponse = serde_json::from_slice(bytes)
        .map_err(|source| GuardrailsAiGuardrailError::ResponseJson { source })?;
    let first_output = parsed
        .outputs
        .first()
        .ok_or(GuardrailsAiGuardrailError::PreCallMissingOutput)?;
    first_output
        .data
        .first()
        .cloned()
        .ok_or(GuardrailsAiGuardrailError::PreCallMissingOutput)
}
