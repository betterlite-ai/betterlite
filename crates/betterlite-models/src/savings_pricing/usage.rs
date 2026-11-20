use std::collections::BTreeMap;

use crate::billed_tokens::BilledTokenUsage;
use crate::cost_usage::modality_projection::CacheCreationTokenDetails;
use crate::cost_usage::{CostUsage, PromptDetails};
use crate::savings_evidence::usage::baseline::{
    baseline_cache_rate_keys, baseline_usage, cache_token_split,
};
use crate::savings_evidence::usage::{LoggedUsage, RawUsageObject, usage_from_spend_log};
use crate::token_cost::prompt_overlap::PromptExtras;
use crate::token_rates::ModelInfo;

// A logged usage record beside the TTL cache-write split B1693's parsed record keeps opaque.
#[derive(Clone, Debug, Default)]
pub struct RequestUsage {
    pub usage: LoggedUsage,
    pub cache_creation_token_details: Option<CacheCreationTokenDetails>,
}

// _usage_from_spend_log: an absent or malformed record is unavailable, never an error.
#[must_use]
pub fn request_usage(usage_object: Option<&RawUsageObject>) -> Option<RequestUsage> {
    Some(RequestUsage {
        usage: usage_from_spend_log(usage_object)?,
        cache_creation_token_details: None,
    })
}

// The prompt usage a B1672/B1675 policy reads, built from the logged copy.
#[must_use]
pub fn cost_usage(source: &RequestUsage) -> CostUsage {
    let usage = &source.usage;
    let details = usage.prompt_tokens_details.as_ref();
    CostUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_details: details.map(|value| PromptDetails {
            text_tokens: Some(value.text_tokens),
            audio_tokens: Some(value.audio_tokens),
            image_tokens: Some(value.image_tokens),
            video_tokens: Some(value.video_tokens),
            web_search_requests: None,
        }),
        completion_details: usage.completion_tokens_details,
        cache_read_tokens: details.map(|value| value.cached_tokens),
        numeric_extras: BTreeMap::new(),
    }
}

// The prompt counters the token policies read beside CostUsage.
#[must_use]
pub fn prompt_extras(source: &RequestUsage) -> PromptExtras {
    let details = source.usage.prompt_tokens_details.as_ref();
    PromptExtras {
        cache_write_tokens: details.map(|value| value.cache_write_tokens),
        cache_creation_tokens: details.map(|value| value.cache_creation_tokens),
        cache_creation_token_details: source.cache_creation_token_details,
        cached_tokens_details: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
    }
}

// The cache counters B1673's savings reader takes.
#[must_use]
pub fn billed_usage(source: &RequestUsage) -> BilledTokenUsage {
    let cost = cost_usage(source);
    let extras = prompt_extras(source);
    let reasoning = source
        .usage
        .completion_tokens_details
        .as_ref()
        .and_then(|value| value.reasoning_tokens);
    let cache_read = cost.cache_read_tokens;
    let cache_write = extras.cache_write_tokens;
    let ttl = extras.cache_creation_token_details;
    let created_5m = ttl.and_then(|value| value.ephemeral_5m_input_tokens);
    let created_1h = ttl.and_then(|value| value.ephemeral_1h_input_tokens);
    BilledTokenUsage {
        detail_reasoning: reasoning,
        top_level_reasoning: None,
        detail_cache_read: cache_read,
        top_level_cache_read: None,
        detail_cache_creation: cache_write,
        top_level_cache_creation: None,
        detail_cached_audio: None,
        detail_cache_creation_5m: created_5m,
        detail_cache_creation_1h: created_1h,
    }
}

// _baseline_usage: a baseline model pays plain input for cache buckets it publishes no rate for.
#[must_use]
pub fn baseline_request_usage(
    source: &RequestUsage,
    baseline_info: Option<&ModelInfo>,
) -> RequestUsage {
    let (cache_read, cache_creation) = cache_token_split(&source.usage);
    if source.usage.prompt_tokens_details.is_none() || (cache_read <= 0 && cache_creation <= 0) {
        return source.clone();
    }
    let (prices_reads, prices_writes) = baseline_cache_rate_keys(baseline_info);
    let reads = if prices_reads { cache_read } else { 0 };
    let writes = if prices_writes { cache_creation } else { 0 };
    if reads == cache_read && writes == cache_creation {
        return source.clone();
    }
    RequestUsage {
        usage: baseline_usage(source.usage.clone(), baseline_info),
        cache_creation_token_details: if writes == 0 {
            None
        } else {
            source.cache_creation_token_details
        },
    }
}

mod tests;
