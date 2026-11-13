use serde::Serialize;

use crate::converse_usage::cache_details::CacheCreationTokenDetails;

pub mod cache_details;
pub mod counts;
pub mod error;
pub mod reasoning;
pub mod transform;
pub mod wire;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub completion_tokens: i64,
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    pub completion_tokens_details: CompletionTokensDetails,
    pub prompt_tokens_details: PromptTokensDetails,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionTokensDetails {
    #[serde(rename = "accepted_prediction_tokens")]
    pub accepted_prediction_tokens: Option<i64>,
    #[serde(rename = "audio_tokens")]
    pub audio_tokens: Option<i64>,
    #[serde(rename = "reasoning_tokens")]
    pub reasoning_tokens: Option<i64>,
    #[serde(rename = "rejected_prediction_tokens")]
    pub rejected_prediction_tokens: Option<i64>,
    #[serde(rename = "text_tokens")]
    pub text_tokens: Option<i64>,
    #[serde(rename = "image_tokens")]
    pub image_tokens: Option<i64>,
    #[serde(rename = "video_tokens")]
    pub video_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptTokensDetails {
    #[serde(rename = "audio_tokens")]
    pub audio_tokens: Option<i64>,
    #[serde(rename = "cache_write_tokens")]
    pub cache_write_tokens: i64,
    #[serde(rename = "cached_tokens")]
    pub cached_tokens: i64,
    #[serde(rename = "text_tokens")]
    pub text_tokens: i64,
    #[serde(rename = "image_tokens")]
    pub image_tokens: Option<i64>,
    #[serde(rename = "video_tokens")]
    pub video_tokens: Option<i64>,
    #[serde(rename = "cache_creation_tokens")]
    pub cache_creation_tokens: i64,
    #[serde(
        rename = "cache_creation_token_details",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
}
