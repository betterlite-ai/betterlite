use std::collections::BTreeMap;

use crate::api::responses_objects::sdk::record;
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

record!(LiteLlmVectorStoreConfig {
    vector_store_name: String => "vector_store_name",
    litellm_params: Value => "litellm_params",
});
record!(LiteLlmManagedVectorStore {
    vector_store_id: String => "vector_store_id",
    custom_llm_provider: String => "custom_llm_provider",
    vector_store_name: String => "vector_store_name",
    vector_store_description: String => "vector_store_description",
    vector_store_metadata: Value => "vector_store_metadata",
    created_at: String => "created_at",
    updated_at: String => "updated_at",
    litellm_credential_name: String => "litellm_credential_name",
    litellm_params: Value => "litellm_params",
    team_id: String => "team_id",
    user_id: String => "user_id",
    is_config: bool => "is_config",
});
record!(ManagedVectorStoreList {
    object: String => "object",
    data: Vec<LiteLlmManagedVectorStore> => "data",
    total_count: i64 => "total_count",
    current_page: i64 => "current_page",
    total_pages: i64 => "total_pages",
});
record!(VectorStoreResultContent {
    text: String => "text",
    type_name: String => "type",
});
record!(SearchOptionalParams {
    query: Value => "query", filters: Value => "filters", max_num_results: i64 => "max_num_results",
    ranking_options: Value => "ranking_options", rewrite_query: bool => "rewrite_query",
});
pub type VectorStoreSearchRequest = SearchOptionalParams;
record!(VectorStoreSearchResult {
    score: serde_json::Number => "score",
    content: Vec<VectorStoreResultContent> => "content",
    file_id: String => "file_id",
    filename: String => "filename",
    attributes: Value => "attributes",
});
record!(VectorStoreSearchResponse {
    object: String => "object",
    search_query: String => "search_query",
    data: Vec<VectorStoreSearchResult> => "data",
});
record!(VectorStoreSearchFailure {
    vector_store_id: String => "vector_store_id",
    custom_llm_provider: String => "custom_llm_provider",
    error: String => "error",
});
record!(VertexSearchDataStoreExtraBody {
    query: String => "query", page_size: i64 => "pageSize", page_token: String => "pageToken",
    offset: i64 => "offset", one_box_page_size: i64 => "oneBoxPageSize",
    page_categories: Vec<Value> => "pageCategories", image_query: Value => "imageQuery",
    filter: String => "filter", canonical_filter: String => "canonicalFilter", order_by: String => "orderBy",
    user_info: Value => "userInfo", language_code: String => "languageCode",
    facet_specs: Vec<Value> => "facetSpecs", boost_spec: Value => "boostSpec", params: Value => "params",
    query_expansion_spec: Value => "queryExpansionSpec", spell_correction_spec: Value => "spellCorrectionSpec",
    user_pseudo_id: String => "userPseudoId", content_search_spec: Value => "contentSearchSpec",
    ranking_expression: String => "rankingExpression", ranking_expression_backend: String => "rankingExpressionBackend",
    safe_search: bool => "safeSearch", user_labels: BTreeMap<String, String> => "userLabels",
    natural_query_understanding: Value => "naturalLanguageQueryUnderstandingSpec",
    search_as_you_type_spec: Value => "searchAsYouTypeSpec", display_spec: Value => "displaySpec",
    crowding_specs: Vec<Value> => "crowdingSpecs", relevance_threshold: String => "relevanceThreshold",
    relevance_score_spec: Value => "relevanceScoreSpec", custom_ranking_params: Value => "customRankingParams",
});
record!(VertexSearchEngineExtraBody {
    query: String => "query", page_size: i64 => "pageSize", page_token: String => "pageToken",
    offset: i64 => "offset", one_box_page_size: i64 => "oneBoxPageSize",
    page_categories: Vec<Value> => "pageCategories", image_query: Value => "imageQuery",
    filter: String => "filter", canonical_filter: String => "canonicalFilter", order_by: String => "orderBy",
    user_info: Value => "userInfo", language_code: String => "languageCode",
    facet_specs: Vec<Value> => "facetSpecs", boost_spec: Value => "boostSpec", params: Value => "params",
    query_expansion_spec: Value => "queryExpansionSpec", spell_correction_spec: Value => "spellCorrectionSpec",
    user_pseudo_id: String => "userPseudoId", content_search_spec: Value => "contentSearchSpec",
    ranking_expression: String => "rankingExpression", ranking_expression_backend: String => "rankingExpressionBackend",
    safe_search: bool => "safeSearch", user_labels: BTreeMap<String, String> => "userLabels",
    natural_query_understanding: Value => "naturalLanguageQueryUnderstandingSpec",
    search_as_you_type_spec: Value => "searchAsYouTypeSpec", display_spec: Value => "displaySpec",
    crowding_specs: Vec<Value> => "crowdingSpecs", relevance_threshold: String => "relevanceThreshold",
    relevance_score_spec: Value => "relevanceScoreSpec", custom_ranking_params: Value => "customRankingParams",
    data_store_specs: Vec<Value> => "dataStoreSpecs", num_results_per_data_store: i64 => "numResultsPerDataStore",
});
record!(CreateOptionalParams {
    name: String => "name", file_ids: Vec<String> => "file_ids",
    expires_after: VectorStoreExpirationPolicy => "expires_after",
    chunking_strategy: VectorStoreChunkingStrategy => "chunking_strategy",
    metadata: BTreeMap<String, String> => "metadata",
});
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorStoreCreateRequest {
    #[serde(flatten)]
    pub optional: CreateOptionalParams,
}
record!(VectorStoreCreateResponse {
    id: String => "id", object: String => "object", created_at: i64 => "created_at",
    name: String => "name", bytes: i64 => "bytes", file_counts: VectorStoreFileCounts => "file_counts",
    status: String => "status", expires_after: VectorStoreExpirationPolicy => "expires_after",
    expires_at: i64 => "expires_at", last_active_at: i64 => "last_active_at",
    metadata: BTreeMap<String, String> => "metadata",
});
record!(VectorStoreExpirationPolicy {
    anchor: String => "anchor", days: i64 => "days",
});
record!(StaticChunkingConfig {
    max_chunk_size_tokens: i64 => "max_chunk_size_tokens",
    chunk_overlap_tokens: i64 => "chunk_overlap_tokens",
});
record!(AutoChunkingStrategy { type_name: String => "type" });
record!(StaticChunkingStrategy {
    type_name: String => "type", static_config: StaticChunkingConfig => "static",
});
record!(union VectorStoreChunkingStrategy [tag = "type"] {
    "auto" => Auto(AutoChunkingStrategy),
    "static" => Static(StaticChunkingStrategy),
});
record!(VectorStoreFileCounts {
    in_progress: i64 => "in_progress", completed: i64 => "completed", failed: i64 => "failed",
    cancelled: i64 => "cancelled", total: i64 => "total",
});
record!(BaseVectorStoreAuthCredentials {
    headers: BTreeMap<String, Value> => "headers", query_params: BTreeMap<String, Value> => "query_params",
});
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
record!(VectorStoreIndexEndpoints {
    read: Vec<(String, String)> => "read", write: Vec<(String, String)> => "write",
});
pub type IndexEndpoints = VectorStoreIndexEndpoints;
record!(BedrockKbLocation {
    type_name: String => "type", s3_location: Value => "s3Location", web_location: Value => "webLocation",
    kendra_document_location: Value => "kendraDocumentLocation", salesforce_location: Value => "salesforceLocation",
    share_point_location: Value => "sharePointLocation", confluence_location: Value => "confluenceLocation",
    custom_document_location: Value => "customDocumentLocation", sql_location: Value => "sqlLocation",
});
record!(BedrockKbRowValue {
    column_name: String => "columnName", column_value: String => "columnValue", type_name: String => "type",
});
record!(BedrockKbContent {
    type_name: String => "type", text: String => "text", byte_content: String => "byteContent",
    row: Vec<BedrockKbRowValue> => "row",
});
record!(BedrockKbRetrievalResult {
    content: BedrockKbContent => "content", location: BedrockKbLocation => "location",
    score: serde_json::Number => "score", metadata: BTreeMap<String, Value> => "metadata",
});
record!(BedrockKbMetadataAttribute {
    description: String => "description", key: String => "key", type_name: String => "type",
});
record!(KbImplicitFilterConfig {
    metadata_attributes: Vec<BedrockKbMetadataAttribute> => "metadataAttributes", model_arn: String => "modelArn",
});
record!(KbSelectiveModeConfig {});
record!(BedrockKbMetadataConfiguration {
    selection_mode: String => "selectionMode", selective_mode_configuration: KbSelectiveModeConfig => "selectiveModeConfiguration",
});
record!(BedrockKbModelConfiguration {
    model_request_fields: Value => "additionalModelRequestFields", model_arn: String => "modelArn",
});
record!(KbRerankingModelConfig {
    model_configuration: BedrockKbModelConfiguration => "modelConfiguration",
    metadata_configuration: BedrockKbMetadataConfiguration => "metadataConfiguration",
});
record!(KbRerankingConfig {
    bedrock_rerank_config: KbRerankingModelConfig => "bedrockRerankingConfiguration",
    type_name: String => "type",
});
record!(KbVectorSearchConfig {
    filter: Value => "filter", implicit_filter_configuration: KbImplicitFilterConfig => "implicitFilterConfiguration",
    number_of_results: i64 => "numberOfResults", override_search_type: String => "overrideSearchType",
    reranking_configuration: KbRerankingConfig => "rerankingConfiguration",
});
record!(KbRetrievalConfig {
    vector_search_configuration: KbVectorSearchConfig => "vectorSearchConfiguration",
});
record!(BedrockKbRetrievalQuery { text: String => "text" });
record!(KbGuardrailConfig {
    guardrail_id: String => "guardrailId", guardrail_version: String => "guardrailVersion",
});
record!(BedrockKbUserContext { user_id: String => "userId" });
record!(BedrockKbRequest {
    guardrail_configuration: KbGuardrailConfig => "guardrailConfiguration", next_token: String => "nextToken",
    retrieval_configuration: KbRetrievalConfig => "retrievalConfiguration",
    retrieval_query: BedrockKbRetrievalQuery => "retrievalQuery", user_context: BedrockKbUserContext => "userContext",
});
record!(BedrockKbResponse {
    guardrail_action: String => "guardrailAction", next_token: String => "nextToken",
    retrieval_results: Vec<BedrockKbRetrievalResult> => "retrievalResults",
});

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
