use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;

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
    Other(OciOtherPart),
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciOtherPart {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: OciFunctionType,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciToolDefinition {
    #[serde(rename = "type", default)]
    pub kind: OciFunctionType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OciMessage {
    pub role: OciRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<OciContentPart>>,
    #[serde(rename = "toolCalls", default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OciToolCall>>,
    #[serde(
        rename = "toolCallId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciChatRequestPayload {
    pub api_format: String,
    pub messages: Vec<OciMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OciToolDefinition>>,
    #[serde(default)]
    pub is_stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_generations: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_probs: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionTokenDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_prediction_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciPromptTokenDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciResponseUsage {
    pub prompt_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<OciCompletionTokenDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<OciPromptTokenDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciResponseChoice {
    pub index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<OciMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciChatResponse {
    pub api_format: String,
    pub time_created: String,
    pub choices: Vec<OciResponseChoice>,
    pub usage: OciResponseUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciCompletionResponse {
    pub model_id: String,
    pub model_version: String,
    pub chat_response: OciChatResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciStreamDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<OciContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OciToolCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OciStreamChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<OciStreamDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}
