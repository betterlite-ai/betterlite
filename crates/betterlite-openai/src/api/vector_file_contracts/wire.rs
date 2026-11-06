use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use super::records::{
    ChunkingStrategy, StaticChunkingConfig, VectorStoreFileAuthCredentials,
    VectorStoreFileContentResponse, VectorStoreFileContentTextPart, VectorStoreFileCreateRequest,
    VectorStoreFileDeleteResponse, VectorStoreFileListQueryParams, VectorStoreFileListResponse,
    VectorStoreFileObject, VectorStoreFileUpdateRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorFileRecordKind {
    FileObject,
    CreateRequest,
    UpdateRequest,
    ListQuery,
    ListResponse,
    DeleteResponse,
    ContentResponse,
    ContentTextPart,
    AuthCredentials,
    ChunkingStrategy,
    StaticChunking,
}

impl TryFrom<u8> for VectorFileRecordKind {
    type Error = VectorFileRecordError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::FileObject),
            1 => Ok(Self::CreateRequest),
            2 => Ok(Self::UpdateRequest),
            3 => Ok(Self::ListQuery),
            4 => Ok(Self::ListResponse),
            5 => Ok(Self::DeleteResponse),
            6 => Ok(Self::ContentResponse),
            7 => Ok(Self::ContentTextPart),
            8 => Ok(Self::AuthCredentials),
            9 => Ok(Self::ChunkingStrategy),
            10 => Ok(Self::StaticChunking),
            _ => Err(VectorFileRecordError::UnknownKind(value)),
        }
    }
}

#[derive(Debug, Error)]
pub enum VectorFileRecordError {
    #[error("unknown vector file record kind: {0}")]
    UnknownKind(u8),
    #[error("vector file record JSON exceeds the limit of {limit} bytes")]
    JsonTooLarge { limit: usize },
    #[error("vector file record JSON exceeds the limit of {limit} items")]
    TooManyItems { limit: usize },
    #[error("invalid vector file record")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum VectorFileRecord {
    FileObject(VectorStoreFileObject),
    CreateRequest(VectorStoreFileCreateRequest),
    UpdateRequest(VectorStoreFileUpdateRequest),
    ListQuery(VectorStoreFileListQueryParams),
    ListResponse(VectorStoreFileListResponse),
    DeleteResponse(VectorStoreFileDeleteResponse),
    ContentResponse(VectorStoreFileContentResponse),
    ContentTextPart(VectorStoreFileContentTextPart),
    AuthCredentials(VectorStoreFileAuthCredentials),
    ChunkingStrategy(ChunkingStrategy),
    StaticChunking(StaticChunkingConfig),
}

fn check_item_limit(root: &Value, max_items: usize) -> Result<(), VectorFileRecordError> {
    let mut pending = vec![root];
    let mut remaining = max_items;
    while let Some(value) = pending.pop() {
        let children: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            Value::Object(members) => members.values().collect(),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
        };
        remaining = remaining
            .checked_sub(children.len())
            .ok_or(VectorFileRecordError::TooManyItems { limit: max_items })?;
        pending.extend(children);
    }
    Ok(())
}

pub fn decode_vector_file_contracts(
    kind: VectorFileRecordKind,
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<VectorFileRecord, VectorFileRecordError> {
    use VectorFileRecord as Record;
    use VectorFileRecordKind as Kind;
    if bytes.len() > limits.max_json_bytes {
        return Err(VectorFileRecordError::JsonTooLarge {
            limit: limits.max_json_bytes,
        });
    }
    let parsed: Value = serde_json::from_slice(bytes)?;
    check_item_limit(&parsed, limits.max_items)?;
    Ok(match kind {
        Kind::FileObject => Record::FileObject(serde_json::from_value(parsed)?),
        Kind::CreateRequest => Record::CreateRequest(serde_json::from_value(parsed)?),
        Kind::UpdateRequest => Record::UpdateRequest(serde_json::from_value(parsed)?),
        Kind::ListQuery => Record::ListQuery(serde_json::from_value(parsed)?),
        Kind::ListResponse => Record::ListResponse(serde_json::from_value(parsed)?),
        Kind::DeleteResponse => Record::DeleteResponse(serde_json::from_value(parsed)?),
        Kind::ContentResponse => Record::ContentResponse(serde_json::from_value(parsed)?),
        Kind::ContentTextPart => Record::ContentTextPart(serde_json::from_value(parsed)?),
        Kind::AuthCredentials => Record::AuthCredentials(serde_json::from_value(parsed)?),
        Kind::ChunkingStrategy => Record::ChunkingStrategy(serde_json::from_value(parsed)?),
        Kind::StaticChunking => Record::StaticChunking(serde_json::from_value(parsed)?),
    })
}
