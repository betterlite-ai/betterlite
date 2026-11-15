use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::alice::error::{AliceError, request_error};
use crate::alice::wire::AliceVerdict;

pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) async fn read_answer(mut response: reqwest::Response) -> Result<Vec<u8>, AliceError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AliceError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) fn verdict_from_body(body: &[u8]) -> Result<AliceVerdict, AliceError> {
    let parsed: GuardrailValue =
        serde_json::from_slice(body).map_err(|source| AliceError::ResponseJson { source })?;
    if !matches!(parsed, GuardrailValue::Object(_)) {
        return Err(AliceError::NonObjectBody);
    }
    serde_json::from_slice(body).map_err(|source| AliceError::MalformedVerdict { source })
}
