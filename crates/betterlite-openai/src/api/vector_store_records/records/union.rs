use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::wire::{VectorStoreRecordError, VectorStoreRecordKind};
use super::basic::VectorStoreToolParams;
use super::bedrock::{BedrockKbRequest, BedrockKbResponse, BedrockKbRowValue};
use super::pydantic::{
    IndexCreateRequest, IndexListResponse, LiteLlmManagedVectorStoreIndex,
    VectorStoreDeleteRequest, VectorStoreInfoRequest, VectorStoreUpdateRequest,
};
use super::vector::{
    IndexEndpoints, LiteLlmManagedVectorStore, ManagedVectorStoreList, VectorStoreCreateRequest,
    VectorStoreCreateResponse, VectorStoreSearchFailure, VectorStoreSearchRequest,
    VectorStoreSearchResponse, VertexSearchDataStoreExtraBody, VertexSearchEngineExtraBody,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VectorStoreRecord {
    ToolParams(VectorStoreToolParams),
    ManagedStore(LiteLlmManagedVectorStore),
    ManagedStoreList(ManagedVectorStoreList),
    UpdateRequest(VectorStoreUpdateRequest),
    DeleteRequest(VectorStoreDeleteRequest),
    InfoRequest(VectorStoreInfoRequest),
    SearchRequest(VectorStoreSearchRequest),
    SearchResponse(VectorStoreSearchResponse),
    SearchFailure(VectorStoreSearchFailure),
    DiscoveryDataStore(VertexSearchDataStoreExtraBody),
    DiscoveryEngine(VertexSearchEngineExtraBody),
    CreateRequest(VectorStoreCreateRequest),
    CreateResponse(VectorStoreCreateResponse),
    IndexCreateRequest(IndexCreateRequest),
    ManagedIndex(LiteLlmManagedVectorStoreIndex),
    IndexListResponse(IndexListResponse),
    IndexEndpoints(IndexEndpoints),
    BedrockRequest(BedrockKbRequest),
    BedrockResponse(BedrockKbResponse),
    BedrockRow(BedrockKbRowValue),
}

impl VectorStoreRecord {
    pub(crate) fn from_value(
        kind: VectorStoreRecordKind,
        value: Value,
    ) -> Result<Self, VectorStoreRecordError> {
        macro_rules! decode {
            ($variant:ident) => {
                Self::$variant(serde_json::from_value(value)?)
            };
        }
        let record = match kind {
            VectorStoreRecordKind::ToolParams => decode!(ToolParams),
            VectorStoreRecordKind::ManagedStore => decode!(ManagedStore),
            VectorStoreRecordKind::ManagedStoreList => decode!(ManagedStoreList),
            VectorStoreRecordKind::UpdateRequest => decode!(UpdateRequest),
            VectorStoreRecordKind::DeleteRequest => decode!(DeleteRequest),
            VectorStoreRecordKind::InfoRequest => decode!(InfoRequest),
            VectorStoreRecordKind::SearchRequest => decode!(SearchRequest),
            VectorStoreRecordKind::SearchResponse => decode!(SearchResponse),
            VectorStoreRecordKind::SearchFailure => decode!(SearchFailure),
            VectorStoreRecordKind::DiscoveryDataStore => decode!(DiscoveryDataStore),
            VectorStoreRecordKind::DiscoveryEngine => decode!(DiscoveryEngine),
            VectorStoreRecordKind::CreateRequest => decode!(CreateRequest),
            VectorStoreRecordKind::CreateResponse => decode!(CreateResponse),
            VectorStoreRecordKind::IndexCreateRequest => decode!(IndexCreateRequest),
            VectorStoreRecordKind::ManagedIndex => decode!(ManagedIndex),
            VectorStoreRecordKind::IndexListResponse => decode!(IndexListResponse),
            VectorStoreRecordKind::IndexEndpoints => decode!(IndexEndpoints),
            VectorStoreRecordKind::BedrockRequest => decode!(BedrockRequest),
            VectorStoreRecordKind::BedrockResponse => decode!(BedrockResponse),
            VectorStoreRecordKind::BedrockRow => decode!(BedrockRow),
        };
        Ok(record)
    }
}
