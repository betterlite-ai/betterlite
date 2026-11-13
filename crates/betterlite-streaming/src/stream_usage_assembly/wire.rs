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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Choice {
    pub finish_reason: Option<String>,
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
    pub prompt_tokens_details: Option<DetailsField<PromptTokensDetails>>,
    pub completion_tokens_details: Option<DetailsField<CompletionTokensDetails>>,
    pub server_tool_use: Option<JsonValue>,
    // A plain Option accepts the null that reaches the untagged usage slot as a unit, and Decimal reads the number's exact text.
    pub cost: Option<Decimal>,
    pub inference_geo: Option<JsonValue>,
    pub speed: Option<JsonValue>,
}

// Usage(**record) drops an empty details object, so a later {} never clears an earlier record while an all-null one does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DetailsField<T> {
    Empty(EmptyDetails),
    Present(T),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyDetails {
    #[serde(skip)]
    marker: (),
}

impl<T> DetailsField<T> {
    pub(crate) fn present(self) -> Option<T> {
        match self {
            Self::Present(details) => Some(details),
            Self::Empty(_) => None,
        }
    }
}

// The wrapper mirrors cache write and cache creation tokens into each other, and reads both from a nested cache count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<i64>,
    pub web_search_requests: Option<i64>,
    pub google_maps_grounding_requests: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
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
    pub reasoning_tokens: Option<i64>,
    pub text_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct CacheCreationTokenDetails {
    pub ephemeral_5m_input_tokens: Option<i64>,
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerToolUse {
    pub web_search_requests: Option<i64>,
    pub tool_search_requests: Option<i64>,
    pub browser_open_requests: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cache_creation_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub server_tool_use: Option<ServerToolUse>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost: Option<Decimal>,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
}
