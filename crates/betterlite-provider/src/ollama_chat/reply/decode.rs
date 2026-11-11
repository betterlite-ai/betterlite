use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;

use super::super::error::OllamaError;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DecodedReply {
    #[serde(default)]
    pub done_reason: Option<String>,
    pub message: ReplyMessage,
    #[serde(default)]
    pub prompt_eval_count: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReplyMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ReplyToolCall>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReplyToolCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    pub function: ReplyFunction,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReplyFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<Json>,
    #[serde(default)]
    pub parameters: Option<Json>,
}

impl TryFrom<&[u8]> for DecodedReply {
    type Error = OllamaError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes).map_err(|source| OllamaError::Shape { source })
    }
}
