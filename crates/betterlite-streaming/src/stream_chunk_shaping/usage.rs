use serde::{Deserialize, Serialize};

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<TokenDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<TokenDetails>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// Both token-detail wrappers hold only counters, so one type carries either field verbatim.
#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct TokenDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_prediction_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_prediction_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_maps_grounding_requests: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens_details: Option<CachedTokensDetails>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct CacheCreationTokenDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct CachedTokensDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<i64>,
    #[serde(flatten)]
    pub extra: JsonObject,
}
