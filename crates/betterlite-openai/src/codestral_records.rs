pub mod chunk;
pub mod error;
pub mod options;
pub mod reply;

use betterlite_http::proxy::limits::TransportLimits;
use uuid::Uuid;

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
