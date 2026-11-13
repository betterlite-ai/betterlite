use super::error::VectorStoreError;
use crate::api::vector_store_records::records::union::VectorStoreRecord;
use crate::api::vector_store_records::wire::{
    VectorStoreRecordError, VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

const MAX_REPLY_BYTES: usize = 0x0100_0000;
const MAX_REPLY_ITEMS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    Search,
    Create,
    File,
}

impl ReplyKind {
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            b'S' => Some(Self::Search),
            b'C' => Some(Self::Create),
            b'F' => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum VectorReply {
    Record(Box<VectorStoreRecord>),
    Json(Value),
}

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("vector store reply exceeds the byte limit")]
    TooLarge,
    #[error("vector store reply is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("vector store reply is not a valid record")]
    Record(#[source] VectorStoreRecordError),
}

pub fn decode_vector_reply(kind: ReplyKind, bytes: &[u8]) -> Result<VectorReply, ReplyError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(ReplyError::TooLarge);
    }
    let record_kind = match kind {
        ReplyKind::Search => VectorStoreRecordKind::SearchResponse,
        ReplyKind::Create => VectorStoreRecordKind::CreateResponse,
        ReplyKind::File => {
            let value = serde_json::from_slice(bytes).map_err(ReplyError::Json)?;
            return Ok(VectorReply::Json(value));
        }
    };
    let limits = WireDecodeLimits {
        max_json_bytes: MAX_REPLY_BYTES,
        max_event_bytes: MAX_REPLY_BYTES,
        max_items: MAX_REPLY_ITEMS,
    };
    let record = decode_vector_store_records(record_kind, bytes, limits);
    record
        .map(|decoded| VectorReply::Record(Box::new(decoded)))
        .map_err(ReplyError::Record)
}

pub fn transform_response(
    kind: ReplyKind,
    response: &http::Response<Vec<u8>>,
) -> Result<VectorReply, VectorStoreError> {
    decode_vector_reply(kind, response.body()).map_err(|source| VectorStoreError::InvalidReply {
        status: response.status(),
        headers: Box::new(response.headers().clone()),
        source: Box::new(source),
    })
}
