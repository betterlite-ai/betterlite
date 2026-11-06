use crate::arguments::json::Json;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};
use thiserror::Error;

// A provider response is already parsed in memory by the time these extractors see it, so the byte parser bounds only the fuzz and replay path.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

// One response object carries only one surface's section, and LiteLLM probes all three, so every section is optional here.
#[serde_as]
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ToolCallResponse {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub content: Vec<AnthropicContentBlock>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ChatChoice {
    pub message: Option<ChatMessage>,
}

#[serde_as]
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ChatMessage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub tool_calls: Vec<ChatToolCall>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ChatToolCall {
    pub id: Option<String>,
    pub function: Option<ChatFunction>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ChatFunction {
    pub name: Option<String>,
    // OpenAI declares a JSON string, and providers also send the object form some clients already decoded.
    pub arguments: Option<Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub name: Option<String>,
    pub call_id: Option<String>,
    pub id: Option<String>,
    pub arguments: Option<Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    // Anthropic declares an object, and a block whose input is not one contributes empty arguments.
    pub input: Option<Json>,
}

#[derive(Debug, Error)]
pub enum ResponseToolCallError {
    #[error("the response body is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the bytes are not a chat, Responses or Anthropic response projection")]
    Envelope {
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_tool_call_response(bytes: &[u8]) -> Result<ToolCallResponse, ResponseToolCallError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ResponseToolCallError::TooLarge {
            length: bytes.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| ResponseToolCallError::Envelope { source })
}

mod tests;
