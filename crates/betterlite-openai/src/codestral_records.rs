pub mod chunk;
pub mod error;
pub mod options;
pub mod reply;

use betterlite_http::proxy::limits::TransportLimits;
use uuid::Uuid;

use crate::codestral_records::error::TextCompletionCodestralError;

mod tests;

pub const MAX_RECORD_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSource {
    pub id: Uuid,
    pub created: i64,
}

// httpx hands LiteLLM a lossy decode of the bytes, so bad UTF-8 never fails here
pub fn record_text(body: &[u8]) -> Result<String, TextCompletionCodestralError> {
    if body.len() > MAX_RECORD_BYTES {
        return Err(TextCompletionCodestralError::TooLarge {
            size: body.len(),
            limit: MAX_RECORD_BYTES,
        });
    }
    let mut text = String::with_capacity(body.len());
    for part in body.utf8_chunks() {
        text.push_str(part.valid());
        text.extend((!part.invalid().is_empty()).then_some('\u{fffd}'));
    }
    Ok(text)
}
