use serde::{Deserialize, Serialize};

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<JsonValue>,
    pub prompt_tokens_details: Option<TokenDetails>,
    pub completion_tokens_details: Option<TokenDetails>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// Both token-detail wrappers hold only counters, so one type carries either field verbatim.
#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TokenDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_token_details: Option<JsonValue>,
    #[serde(flatten)]
    pub extra: JsonObject,
}
