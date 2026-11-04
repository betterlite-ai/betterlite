use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::wire::{VectorStoreRecordError, VectorStoreRecordKind};

pub type WireRecord = BTreeMap<String, Value>;

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
        if let Some(value) = self.filters.as_ref().filter(|value| !value.is_null()) {
            result.insert("filters".to_owned(), value.clone());
        }
        if let Some(value) = self.max_num_results {
            result.insert("max_num_results".to_owned(), Value::from(value));
        }
        if let Some(value) = self
            .ranking_options
            .as_ref()
            .filter(|value| !value.is_null())
        {
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

pub type SupportedIntegrations = VectorStoreIntegration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreOpenAiParam {
    Filters,
    MaxNumResults,
    RankingOptions,
    RewriteQuery,
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

macro_rules! wire_record {
    ($record:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
        #[serde(transparent)]
        pub struct $record(pub WireRecord);
    };
}

wire_record!(LiteLlmVectorStoreConfig);
wire_record!(LiteLlmManagedVectorStore);
wire_record!(ManagedVectorStoreList);
wire_record!(VectorStoreResultContent);
wire_record!(SearchOptionalParams);
wire_record!(VectorStoreSearchRequest);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreUpdateRequest {
    pub vector_store_id: String,
    pub custom_llm_provider: Option<String>,
    pub vector_store_name: Option<String>,
    pub vector_store_description: Option<String>,
    pub vector_store_metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreDeleteRequest {
    pub vector_store_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreInfoRequest {
    pub vector_store_id: String,
}
wire_record!(VectorStoreSearchResult);
wire_record!(VectorStoreSearchResponse);
wire_record!(VectorStoreSearchFailure);
wire_record!(VertexSearchDataStoreExtraBody);
wire_record!(VertexSearchEngineExtraBody);
wire_record!(VectorStoreCreateRequest);
wire_record!(VectorStoreCreateResponse);
wire_record!(VectorStoreExpirationPolicy);
wire_record!(AutoChunkingStrategy);
wire_record!(StaticChunkingConfig);
wire_record!(StaticChunkingStrategy);
wire_record!(VectorStoreChunkingStrategy);
wire_record!(VectorStoreFileCounts);
wire_record!(CreateOptionalParams);
wire_record!(BaseVectorStoreAuthCredentials);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCreateLiteLlmParams {
    pub vector_store_index: String,
    pub vector_store_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCreateRequest {
    pub index_name: String,
    pub litellm_params: IndexCreateLiteLlmParams,
    pub index_info: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmManagedVectorStoreIndex {
    pub id: String,
    pub index_name: String,
    pub litellm_params: IndexCreateLiteLlmParams,
    pub index_info: Option<Value>,
    pub created_at: Option<String>,
    pub created_by: Option<String>,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexListObject {
    #[default]
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexListResponse {
    #[serde(default)]
    pub object: IndexListObject,
    pub data: Vec<LiteLlmManagedVectorStoreIndex>,
}
wire_record!(IndexEndpoints);
wire_record!(VectorStoreIndexEndpoints);
wire_record!(BedrockKbLocation);
wire_record!(BedrockKbContent);
wire_record!(BedrockKbRetrievalResult);
wire_record!(BedrockKbMetadataAttribute);
wire_record!(KbImplicitFilterConfig);
wire_record!(KbSelectiveModeConfig);
wire_record!(BedrockKbMetadataConfiguration);
wire_record!(BedrockKbModelConfiguration);
wire_record!(KbRerankingConfig);
wire_record!(KbVectorSearchConfig);
wire_record!(KbRetrievalConfig);
wire_record!(BedrockKbRetrievalQuery);
wire_record!(KbGuardrailConfig);
wire_record!(BedrockKbUserContext);
wire_record!(BedrockKbRequest);
wire_record!(BedrockKbResponse);
wire_record!(BedrockKbRowValue);

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
    pub(super) fn from_value(
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
