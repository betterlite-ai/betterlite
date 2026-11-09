use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

// LiteLLM reads a plain dict chunk and a ModelResponse/ModelResponseStream chunk by different rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkForm {
    #[default]
    Dict,
    ModelResponse,
    ModelResponseStream,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Chunk {
    pub form: ChunkForm,
    pub usage: UsageSlot,
    #[serde(rename = "_hidden_params")]
    pub hidden_params: HiddenParams,
    pub choices: Vec<Choice>,
}

// An explicit null usage key stops the hidden-usage lookup on an SDK object, while an absent key falls through to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum UsageSlot {
    Key(Option<Box<UsageRecord>>),
    #[default]
    Absent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HiddenParams {
    pub usage: Option<UsageRecord>,
    pub custom_llm_provider: Option<String>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Choice {
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct UsageRecord {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub prompt_cache_hit_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    pub server_tool_use: Option<JsonValue>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost: Option<Decimal>,
    pub inference_geo: Option<JsonValue>,
    pub speed: Option<JsonValue>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// The wrapper mirrors cache write and cache creation tokens into each other, and reads both from a nested cache count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub google_maps_grounding_requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

impl PromptTokensDetails {
    #[must_use]
    pub fn mirrored(mut self) -> Self {
        let nested = self
            .extra
            .get("cache_creation_input_tokens")
            .and_then(JsonValue::as_i64);
        let write = self
            .cache_write_tokens
            .or(self.cache_creation_tokens)
            .or(nested);
        if let Some(count) = write {
            self.cache_write_tokens = Some(count);
            self.cache_creation_tokens = Some(count);
        }
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct CompletionTokensDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_prediction_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_prediction_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct CacheCreationTokenDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ReasoningChoice {
    pub message: ReasoningMessage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ReasoningMessage {
    pub reasoning_content: Option<String>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerToolUse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_search_requests: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_open_requests: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<ServerToolUse>,
    #[serde(
        with = "rust_decimal::serde::arbitrary_precision_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_geo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
}
