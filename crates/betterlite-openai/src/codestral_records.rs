pub mod chunk;
pub mod error;
pub mod options;
pub mod reply;

use std::str;

use betterlite_http::proxy::limits::TransportLimits;
use uuid::Uuid;

use crate::codestral_records::error::TextCompletionCodestralError;

mod tests;

pub const MAX_RECORD_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

// The fresh id and clock a response record uses when the provider left them out
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSource {
    pub id: Uuid,
    pub created: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Reply,
    Chunk,
}

impl RecordKind {
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            b'R' => Some(Self::Reply),
            b'C' => Some(Self::Chunk),
            _ => None,
        }
    }
}

pub fn record_text(body: &[u8]) -> Result<&str, TextCompletionCodestralError> {
    if body.len() > MAX_RECORD_BYTES {
        return Err(TextCompletionCodestralError::TooLarge {
            size: body.len(),
            limit: MAX_RECORD_BYTES,
        });
    }
    str::from_utf8(body).map_err(TextCompletionCodestralError::BodyNotUtf8)
}
