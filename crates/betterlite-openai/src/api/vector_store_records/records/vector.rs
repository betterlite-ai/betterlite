use std::collections::BTreeMap;

use crate::api::responses_objects::sdk::record;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
record!(VectorStoreIndexEndpoints {
    read: Vec<(String, String)> => "read", write: Vec<(String, String)> => "write",
});
pub type IndexEndpoints = VectorStoreIndexEndpoints;
