use std::collections::BTreeMap;

use crate::api::responses_objects::sdk::record;
use serde_json::Value;

record!(BedrockKbLocation {
    type_name: String => "type", s3_location: Value => "s3Location", web_location: Value => "webLocation",
    kendra_document_location: Value => "kendraDocumentLocation", salesforce_location: Value => "salesforceLocation",
    share_point_location: Value => "sharePointLocation", confluence_location: Value => "confluenceLocation",
    custom_document_location: Value => "customDocumentLocation", sql_location: Value => "sqlLocation",
});
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BedrockKbRowValue {
    #[serde(rename = "columnName")]
    pub column_name: String,
    #[serde(rename = "columnValue")]
    pub column_value: String,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
record!(BedrockKbContent {
    type_name: String => "type", text: String => "text", byte_content: String => "byteContent",
    row: Vec<BedrockKbRowValue> => "row" [strict],
});
record!(BedrockKbRetrievalResult {
    content: BedrockKbContent => "content" [strict], location: BedrockKbLocation => "location",
    score: super::score::Score => "score" [strict], metadata: BTreeMap<String, Value> => "metadata",
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BedrockKbUserContext {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
record!(BedrockKbRequest {
    guardrail_configuration: KbGuardrailConfig => "guardrailConfiguration", next_token: String => "nextToken",
    retrieval_configuration: KbRetrievalConfig => "retrievalConfiguration",
    retrieval_query: BedrockKbRetrievalQuery => "retrievalQuery",
    user_context: BedrockKbUserContext => "userContext" [strict],
});
record!(BedrockKbResponse {
    guardrail_action: super::basic::GuardrailAction => "guardrailAction", next_token: String => "nextToken",
    retrieval_results: Vec<BedrockKbRetrievalResult> => "retrievalResults" [strict],
});
