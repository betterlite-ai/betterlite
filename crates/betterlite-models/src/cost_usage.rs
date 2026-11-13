use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod aggregation_projection;
pub mod billing_counters;
pub mod error;
mod field;
pub mod interactions;
pub mod modality_projection;
pub mod server_tool_projection;
mod token_count;
pub mod tool_projection;
pub mod transcription;

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CostUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_details: Option<PromptDetails>,
    pub completion_details: Option<CompletionDetails>,
    pub cache_read_tokens: Option<i64>,
    pub numeric_extras: BTreeMap<String, Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PromptDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub web_search_requests: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CompletionDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

mod tests;
