use std::collections::BTreeMap;

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use thiserror::Error;

pub type WireRecord = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VectorStoreRecordKind {
    ToolParams = 0,
    ManagedStore = 1,
    ManagedStoreList = 2,
    UpdateRequest = 3,
    DeleteRequest = 4,
    InfoRequest = 5,
    SearchRequest = 6,
    SearchResponse = 7,
    SearchFailure = 8,
    DiscoveryDataStore = 9,
    DiscoveryEngine = 10,
    CreateRequest = 11,
    CreateResponse = 12,
    IndexCreateRequest = 13,
    ManagedIndex = 14,
    IndexListResponse = 15,
    IndexEndpoints = 16,
    BedrockRequest = 17,
    BedrockResponse = 18,
    BedrockRow = 19,
}

impl TryFrom<u8> for VectorStoreRecordKind {
    type Error = VectorStoreRecordError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::ToolParams),
            1 => Ok(Self::ManagedStore),
            2 => Ok(Self::ManagedStoreList),
            3 => Ok(Self::UpdateRequest),
            4 => Ok(Self::DeleteRequest),
            5 => Ok(Self::InfoRequest),
            6 => Ok(Self::SearchRequest),
            7 => Ok(Self::SearchResponse),
            8 => Ok(Self::SearchFailure),
            9 => Ok(Self::DiscoveryDataStore),
            10 => Ok(Self::DiscoveryEngine),
            11 => Ok(Self::CreateRequest),
            12 => Ok(Self::CreateResponse),
            13 => Ok(Self::IndexCreateRequest),
            14 => Ok(Self::ManagedIndex),
            15 => Ok(Self::IndexListResponse),
            16 => Ok(Self::IndexEndpoints),
            17 => Ok(Self::BedrockRequest),
            18 => Ok(Self::BedrockResponse),
            19 => Ok(Self::BedrockRow),
            _ => Err(VectorStoreRecordError::UnknownKind(value)),
        }
    }
}

#[derive(Debug, Error)]
pub enum VectorStoreRecordError {
    #[error("unknown vector store record kind: {0}")]
    UnknownKind(u8),
    #[error("vector store record JSON exceeds byte limit")]
    JsonTooLarge,
    #[error("vector store record nesting limit exceeded")]
    TooDeep,
    #[error("vector store record item limit exceeded")]
    TooManyItems,
    #[error("invalid vector store record: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Field required: {0}")]
    MissingField(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

pub fn decode_vector_store_records(
    kind: VectorStoreRecordKind,
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<Value, VectorStoreRecordError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(VectorStoreRecordError::JsonTooLarge);
    }
    let root: &RawValue = serde_json::from_slice(bytes)?;
    let mut pending = vec![(root, 0_usize)];
    let mut remaining = limits.max_items;
    while let Some((value, depth)) = pending.pop() {
        if depth > 128 {
            return Err(VectorStoreRecordError::TooDeep);
        }
        let children = match value.get().as_bytes().first() {
            Some(b'[') => serde_json::from_str::<Vec<&RawValue>>(value.get())?,
            Some(b'{') => serde_json::from_str::<Members<'_>>(value.get())?
                .0
                .into_iter()
                .map(|(_, child)| child)
                .collect(),
            _ => Vec::new(),
        };
        remaining = remaining
            .checked_sub(children.len())
            .ok_or(VectorStoreRecordError::TooManyItems)?;
        pending.extend(
            children
                .into_iter()
                .map(|child| (child, depth.saturating_add(1))),
        );
    }
    let value: Value = serde_json::from_slice(bytes)?;
    validate_model(kind, &value)?;
    Ok(value)
}

#[serde_with::serde_as]
#[derive(Deserialize)]
#[serde(transparent)]
struct Members<'input>(
    #[serde_as(as = "serde_with::Map<_, _>")]
    #[serde(borrow)]
    Vec<(IgnoredAny, &'input RawValue)>,
);

fn validate_model(
    kind: VectorStoreRecordKind,
    value: &Value,
) -> Result<(), VectorStoreRecordError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let required: &[&str] = match kind {
        VectorStoreRecordKind::UpdateRequest
        | VectorStoreRecordKind::DeleteRequest
        | VectorStoreRecordKind::InfoRequest => &["vector_store_id"],
        VectorStoreRecordKind::IndexCreateRequest => &["index_name", "litellm_params"],
        VectorStoreRecordKind::ManagedIndex => &["id", "index_name", "litellm_params"],
        VectorStoreRecordKind::IndexListResponse => &["data"],
        VectorStoreRecordKind::SearchFailure => &["vector_store_id", "error"],
        VectorStoreRecordKind::BedrockRow => &["columnName", "columnValue", "type"],
        VectorStoreRecordKind::ToolParams
        | VectorStoreRecordKind::ManagedStore
        | VectorStoreRecordKind::ManagedStoreList
        | VectorStoreRecordKind::SearchRequest
        | VectorStoreRecordKind::SearchResponse
        | VectorStoreRecordKind::DiscoveryDataStore
        | VectorStoreRecordKind::DiscoveryEngine
        | VectorStoreRecordKind::CreateRequest
        | VectorStoreRecordKind::CreateResponse
        | VectorStoreRecordKind::IndexEndpoints
        | VectorStoreRecordKind::BedrockRequest
        | VectorStoreRecordKind::BedrockResponse => &[],
    };
    if let Some(missing) = required.iter().find(|field| !object.contains_key(**field)) {
        return Err(VectorStoreRecordError::MissingField((*missing).to_owned()));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorStoreToolParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_num_results: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_options: Option<Value>,
}

impl VectorStoreToolParams {
    #[must_use]
    pub fn to_dict(&self) -> WireRecord {
        let mut result = WireRecord::new();
        if let Some(value) = &self.filters {
            result.insert("filters".to_owned(), value.clone());
        }
        if let Some(value) = self.max_num_results {
            result.insert("max_num_results".to_owned(), Value::from(value));
        }
        if let Some(value) = &self.ranking_options {
            result.insert("ranking_options".to_owned(), value.clone());
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreIntegration {
    Bedrock,
    Ragflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreIndexType {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreSearchFailureMode {
    Annotate,
    Error,
}

pub type LiteLlmVectorStoreConfig = WireRecord;
pub type LiteLlmManagedVectorStore = WireRecord;
pub type ManagedVectorStoreList = WireRecord;
pub type VectorStoreUpdateRequest = WireRecord;
pub type VectorStoreDeleteRequest = WireRecord;
pub type VectorStoreInfoRequest = WireRecord;
pub type VectorStoreSearchResult = WireRecord;
pub type VectorStoreSearchResponse = WireRecord;
pub type VectorStoreSearchFailure = WireRecord;
pub type VertexSearchDataStoreExtraBody = WireRecord;
pub type VertexSearchEngineExtraBody = WireRecord;
pub type VectorStoreCreateRequest = WireRecord;
pub type VectorStoreCreateResponse = WireRecord;
pub type IndexCreateRequest = WireRecord;
pub type LiteLlmManagedVectorStoreIndex = WireRecord;
pub type IndexListResponse = WireRecord;
pub type IndexEndpoints = WireRecord;
pub type BedrockKbRequest = WireRecord;
pub type BedrockKbResponse = WireRecord;
pub type BedrockKbRowValue = WireRecord;
mod tests;
