use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::response::{IncompleteDetails, ResponseTools};
use betterlite_openai::api::responses_objects::{ResponsesApiResponse, ResponsesApiTruncation};
use betterlite_openai::api::responses_shapes::FunctionCallStatus;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::api::responses_output_details::{ChatAnnotation, ImageItem};
use crate::api::responses_tool_namespaces::records::CacheWrite;

// only the two request keys assembly reads; every other Responses parameter belongs to B908
#[derive(Debug, Deserialize)]
pub struct RequestFields {
    #[serde(default)]
    pub tool_choice: Option<Json>,
    #[serde(default)]
    pub tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub created: i64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<Json>,
    #[serde(default)]
    pub error: Option<BTreeMap<String, Json>>,
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub metadata: Option<Option<BTreeMap<String, Json>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub parallel_tool_calls: Option<Option<bool>>,
    #[serde(default)]
    pub temperature: Option<Json>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub tools: Option<Option<ResponseTools>>,
    #[serde(default)]
    pub top_p: Option<Json>,
    #[serde(default)]
    pub max_output_tokens: Option<i64>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub truncation: Option<ResponsesApiTruncation>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub hidden_params: Option<BTreeMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    #[serde(default)]
    pub finish_reason: Option<String>,
    pub message: ChatMessage,
}

#[derive(Debug, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageItem>,
    #[serde(default)]
    pub annotations: Vec<ChatAnnotation>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub thinking_blocks: Vec<IndexMap<String, Json>>,
    #[serde(default)]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub tool_calls: Vec<ChatToolCall>,
}

#[derive(Debug, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub function: Option<ChatFunction>,
    #[serde(default)]
    pub provider_specific_fields: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub struct ChatFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<Json>,
    #[serde(default)]
    pub status: Option<FunctionCallStatus>,
    #[serde(default)]
    pub provider_specific_fields: Option<Json>,
}

#[derive(Debug)]
pub struct AssembledResponse {
    pub response: ResponsesApiResponse,
    pub hidden_params: BTreeMap<String, Json>,
    pub cache_writes: Vec<CacheWrite>,
}
