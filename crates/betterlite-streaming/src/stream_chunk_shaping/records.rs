use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::stream_chunk_shaping::usage::Usage;

pub type JsonObject = BTreeMap<String, JsonValue>;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(JsonObject),
}

impl JsonValue {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(value) => value.as_i64(),
            Self::Null | Self::Bool(_) | Self::String(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }
}

// Python's len() over a provider value: a list or object counts its members and a string its characters.
#[must_use]
pub fn item_count(value: &JsonValue) -> usize {
    match value {
        JsonValue::Array(items) => items.len(),
        JsonValue::Object(fields) => fields.len(),
        JsonValue::String(text) => text.chars().count(),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => 0,
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamChunk {
    pub id: String,
    #[serde(default = "default_object")]
    pub object: String,
    pub created: i64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    pub choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub provider_specific_fields: Option<JsonObject>,
    pub hidden_params: JsonObject,
    #[serde(flatten)]
    pub extra: JsonObject,
}

fn default_object() -> String {
    "chat.completion.chunk".to_owned()
}

impl Default for StreamChunk {
    fn default() -> Self {
        Self {
            id: String::new(),
            object: default_object(),
            created: 0,
            model: String::new(),
            system_fingerprint: None,
            choices: Vec::new(),
            usage: None,
            provider_specific_fields: None,
            hidden_params: JsonObject::new(),
            extra: JsonObject::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamChoice {
    pub index: i64,
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
    pub logprobs: Option<JsonValue>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamDelta {
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<JsonValue>,
    pub tool_calls: Option<JsonValue>,
    pub audio: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<JsonValue>,
    pub provider_specific_fields: Option<JsonObject>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// The keyword arguments LiteLLM forwards into ModelResponseStream for one provider chunk.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ChunkArguments {
    pub id: Option<String>,
    pub created: Option<i64>,
    pub system_fingerprint: Option<String>,
    pub choices: Option<Vec<StreamChoice>>,
    pub usage: Option<Usage>,
    pub provider_specific_fields: Option<JsonObject>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ChunkResponseObject {
    pub original_chunk: Option<StreamChunk>,
    pub provider_specific_fields: Option<JsonObject>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct GenericChunk {
    pub text: String,
    #[serde(default)]
    pub tool_use: Option<JsonValue>,
    pub is_finished: bool,
    pub finish_reason: String,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub provider_specific_fields: Option<JsonObject>,
}

// frozenset(GenericStreamingChunk.__annotations__): the keys a provider may put in a generic chunk.
pub const GENERIC_CHUNK_FIELDS: [&str; 7] = [
    "text",
    "tool_use",
    "is_finished",
    "finish_reason",
    "usage",
    "index",
    "provider_specific_fields",
];

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ParsedChunkHiddenParams {
    pub provider_specific_fields: Option<JsonObject>,
}
