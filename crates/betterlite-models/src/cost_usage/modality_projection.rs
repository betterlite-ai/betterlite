use rust_decimal::Decimal;
use serde::Deserialize;

use super::PromptDetails;

// Mirrors utils.py's CachedTokensDetails: a nested, per-modality cache breakdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct CachedTokenCounters {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
}

// Mirrors types/utils.py's CacheCreationTokenDetails: ephemeral cache-write TTL buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct CacheCreationTokenDetails {
    pub ephemeral_5m_input_tokens: Option<i64>,
    pub ephemeral_1h_input_tokens: Option<i64>,
}

// Extends CostUsage's prompt billing projection with the counters modality_cost needs, without touching PromptDetails itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PromptModalityUsage {
    pub base: PromptDetails,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
    pub cached_tokens_details: Option<CachedTokenCounters>,
    pub character_count: Option<i64>,
    pub image_count: Option<i64>,
    pub video_length_seconds: Option<Decimal>,
    pub audio_length_seconds: Option<Decimal>,
    pub query_count: Option<i64>,
}

// A nonzero cache_write_tokens wins; otherwise fall through to cache_creation_tokens, then zero.
#[must_use]
pub fn resolved_cache_creation_tokens(usage: &PromptModalityUsage) -> i64 {
    match usage.cache_write_tokens {
        Some(value) if value != 0 => value,
        _ => usage.cache_creation_tokens.unwrap_or(0),
    }
}

mod tests;
