use crate::ocr_error::{OcrError, OcrTransportError};
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
pub(super) fn declared_too_large(response: &StreamResponse, limit: usize) -> bool {
    response
        .response
        .content_length()
        .is_some_and(|declared| declared > u64::try_from(limit).unwrap_or(u64::MAX))
}
pub(super) async fn read_body(
    response: &mut StreamResponse,
    limit: usize,
    success: bool,
) -> Result<Vec<u8>, OcrError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            return OcrError::Transport(OcrTransportError::Timeout);
        }
        OcrError::Transport(OcrTransportError::Failed(TransportError::Body(error)))
    })? {
        let remaining = limit.saturating_sub(body.len());
        if success && chunk.len() > remaining {
            return Err(OcrError::TooLarge { limit });
        }
        body.extend(chunk.iter().take(remaining));
        if !success && body.len() == limit {
            break;
        }
    }
    Ok(body)
}
// an error body is shown to the caller, so bytes that are not UTF-8 are escaped rather than replaced
pub(super) fn printable_body(bytes: &[u8]) -> String {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => text,
        Err(error) => error.as_bytes().escape_ascii().to_string(),
    }
}
