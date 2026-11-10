use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::skip_serializing_none;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OciVendor {
    Cohere,
    Generic,
}
impl OciVendor {
    #[must_use]
    pub const fn api_format(self) -> &'static str {
        match self {
            Self::Cohere => "COHERE",
            Self::Generic => "GENERIC",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OciRole {
    System,
    User,
    Assistant,
    Tool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OciFunctionType {
    #[default]
    #[serde(rename = "FUNCTION")]
    Function,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OciContentPart {
    Text(OciTextPart),
    Image(OciImagePart),
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciTextPart {
    #[serde(rename = "type", default)]
    pub kind: OciTextType,
    pub text: String,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OciTextType {
    #[default]
    #[serde(rename = "TEXT")]
    Text,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciImagePart {
    #[serde(rename = "type", default)]
    pub kind: OciImageType,
    #[serde(rename = "imageUrl")]
    pub image_url: OciImageUrl,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OciImageType {
    #[default]
    #[serde(rename = "IMAGE")]
    Image,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciImageUrl {
    pub url: String,
    pub detail: Option<OciImageDetail>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum OciImageDetail {
    #[serde(rename = "AUTO")]
    Auto,
    #[serde(rename = "HIGH")]
    High,
    #[serde(rename = "LOW")]
    Low,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciToolCall {
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: OciFunctionType,
    pub name: String,
    pub arguments: String,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciToolDefinition {
    #[serde(rename = "type", default)]
    pub kind: OciFunctionType,
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<Json>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciMessage {
    pub role: OciRole,
    pub content: Option<Vec<OciContentPart>>,
    #[serde(rename = "toolCalls")]
    pub tool_calls: Option<Vec<OciToolCall>>,
    #[serde(rename = "toolCallId")]
    pub tool_call_id: Option<String>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciChatRequestPayload {
    pub api_format: String,
    pub messages: Vec<OciMessage>,
    pub tools: Option<Vec<OciToolDefinition>>,
    #[serde(default)]
    pub is_stream: bool,
    pub num_generations: Option<i64>,
    pub max_tokens: Option<i64>,
    pub max_completion_tokens: Option<i64>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub stop: Option<Vec<String>>,
    pub seed: Option<i64>,
    pub frequency_penalty: Option<Number>,
    pub presence_penalty: Option<Number>,
    pub reasoning_effort: Option<String>,
    pub response_format: Option<Object>,
    pub tool_choice: Option<Json>,
    pub logit_bias: Option<Object>,
    pub log_probs: Option<i64>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionTokenDetails {
    pub accepted_prediction_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciPromptTokenDetails {
    pub cached_tokens: Option<u64>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciResponseUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: Option<u64>,
    pub total_tokens: u64,
    pub completion_tokens_details: Option<OciCompletionTokenDetails>,
    pub prompt_tokens_details: Option<OciPromptTokenDetails>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciResponseChoice {
    pub index: i64,
    pub message: Option<OciMessage>,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Object>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciChatResponse {
    pub api_format: String,
    pub time_created: String,
    pub choices: Vec<OciResponseChoice>,
    pub usage: OciResponseUsage,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionResponse {
    pub model_id: String,
    pub model_version: String,
    pub chat_response: OciChatResponse,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciStreamDelta {
    pub content: Option<Vec<OciContentPart>>,
    pub role: Option<String>,
    pub tool_calls: Option<Vec<OciToolCall>>,
}
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciStreamChunk {
    pub finish_reason: Option<String>,
    pub message: Option<OciStreamDelta>,
    pub pad: Option<String>,
    pub index: Option<u32>,
}
