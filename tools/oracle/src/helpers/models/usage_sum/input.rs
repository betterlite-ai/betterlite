use std::collections::BTreeMap;

use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) usages: Vec<UsageInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UsageInput {
    #[serde(default)]
    pub(super) prompt_tokens: Option<i64>,
    #[serde(default)]
    pub(super) completion_tokens: Option<i64>,
    #[serde(default)]
    pub(super) total_tokens: Option<i64>,
    #[serde(default)]
    pub(super) prompt_tokens_details: Option<PromptInput>,
    #[serde(default)]
    pub(super) completion_tokens_details: Option<CompletionInput>,
    #[serde(default)]
    pub(super) numeric_extras: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PromptInput {
    #[serde(default)]
    pub(super) text_tokens: Option<i64>,
    #[serde(default)]
    pub(super) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(super) image_tokens: Option<i64>,
    #[serde(default)]
    pub(super) video_tokens: Option<i64>,
    #[serde(default)]
    pub(super) cache_write_tokens: Option<i64>,
    #[serde(default)]
    pub(super) cache_creation_tokens: Option<i64>,
    #[serde(default)]
    pub(super) cached_tokens: Option<i64>,
    #[serde(default)]
    pub(super) web_search_requests: Option<i64>,
    #[serde(default)]
    pub(super) google_maps_grounding_requests: Option<i64>,
    #[serde(default)]
    pub(super) tool_use_tokens: Option<i64>,
    #[serde(default)]
    pub(super) character_count: Option<i64>,
    #[serde(default)]
    pub(super) image_count: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(super) video_length_seconds: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(super) audio_length_seconds: Option<Decimal>,
    #[serde(default)]
    pub(super) query_count: Option<i64>,
    #[serde(default)]
    pub(super) cached_tokens_details: Option<CachedInput>,
    // Accepted for schema symmetry with LiteLLM's input but always dropped: usage_sum never models the cache-creation TTL split.
    #[serde(default, rename = "cache_creation_token_details")]
    pub(super) _cache_creation_token_details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CachedInput {
    #[serde(default, rename = "text_tokens")]
    pub(super) text: Option<i64>,
    #[serde(default)]
    pub(super) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(super) image_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompletionInput {
    #[serde(default, rename = "text_tokens")]
    pub(super) text: Option<i64>,
    #[serde(default)]
    pub(super) reasoning_tokens: Option<i64>,
    #[serde(default)]
    pub(super) accepted_prediction_tokens: Option<i64>,
    #[serde(default)]
    pub(super) rejected_prediction_tokens: Option<i64>,
    #[serde(default)]
    pub(super) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(super) image_tokens: Option<i64>,
    #[serde(default)]
    pub(super) video_tokens: Option<i64>,
}

impl From<&PromptInput> for PromptTokensDetails {
    fn from(value: &PromptInput) -> Self {
        Self {
            text_tokens: value.text_tokens,
            audio_tokens: value.audio_tokens,
            image_tokens: value.image_tokens,
            video_tokens: value.video_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_creation_tokens: value.cache_creation_tokens,
            cached_tokens: value.cached_tokens,
            cached_tokens_details: value
                .cached_tokens_details
                .as_ref()
                .map(CachedTokensDetails::from),
            web_search_requests: value.web_search_requests,
            google_maps_grounding_requests: value.google_maps_grounding_requests,
            tool_use_tokens: value.tool_use_tokens,
            character_count: value.character_count,
            image_count: value.image_count,
            video_length_seconds: value.video_length_seconds,
            audio_length_seconds: value.audio_length_seconds,
            query_count: value.query_count,
        }
    }
}

impl From<&CachedInput> for CachedTokensDetails {
    fn from(value: &CachedInput) -> Self {
        Self {
            text_tokens: value.text,
            audio_tokens: value.audio_tokens,
            image_tokens: value.image_tokens,
        }
    }
}

impl From<&CompletionInput> for CompletionTokensDetails {
    fn from(value: &CompletionInput) -> Self {
        Self {
            text_tokens: value.text,
            reasoning_tokens: value.reasoning_tokens,
            accepted_prediction_tokens: value.accepted_prediction_tokens,
            rejected_prediction_tokens: value.rejected_prediction_tokens,
            audio_tokens: value.audio_tokens,
            image_tokens: value.image_tokens,
            video_tokens: value.video_tokens,
        }
    }
}
