use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

use super::{LoggedUsage, PromptTokensDetails};

// Unlike the logged extractors, this typed split prefers cache_creation_tokens over cache_write_tokens.
#[must_use]
pub const fn cache_token_split(usage: &LoggedUsage) -> (i64, i64) {
    let Some(details) = &usage.prompt_tokens_details else {
        return (0, 0);
    };
    let created = if details.cache_creation_tokens != 0 {
        details.cache_creation_tokens
    } else {
        details.cache_write_tokens
    };
    (details.cached_tokens, created)
}

// An absent baseline model keeps both buckets priced; a model with no rate key converts them below instead.
#[must_use]
pub fn baseline_cache_rate_keys(baseline_info: Option<&ModelInfo>) -> (bool, bool) {
    let Some(info) = baseline_info else {
        return (true, true);
    };
    (
        scalar_is_nonzero(info.get("cache_read_input_token_cost")),
        scalar_is_nonzero(info.get("cache_creation_input_token_cost")),
    )
}

fn scalar_is_nonzero(scalar: Option<&Scalar>) -> bool {
    match scalar {
        None | Some(Scalar::Null) => false,
        Some(Scalar::Bool(flag)) => *flag,
        Some(Scalar::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).map_or(true, |decimal| !decimal.is_zero())
        }
        Some(Scalar::Text(text)) => !text.is_empty(),
        Some(Scalar::Array(items)) => !items.is_empty(),
        Some(Scalar::Object(_)) => true,
    }
}

// Converts an unpriced cache bucket to ordinary input; leaves usage untouched when nothing changes.
#[must_use]
pub fn baseline_usage(usage: LoggedUsage, baseline_info: Option<&ModelInfo>) -> LoggedUsage {
    let (cache_read, cache_creation) = cache_token_split(&usage);
    if cache_read <= 0 && cache_creation <= 0 {
        return usage;
    }
    let Some(details) = usage.prompt_tokens_details.clone() else {
        return usage;
    };
    let (prices_reads, prices_writes) = baseline_cache_rate_keys(baseline_info);
    let reads = if prices_reads { cache_read } else { 0 };
    let writes = if prices_writes { cache_creation } else { 0 };
    if reads == cache_read && writes == cache_creation {
        return usage;
    }
    rebuild_baseline_usage(&usage, &details, reads, writes)
}

fn rebuild_baseline_usage(
    usage: &LoggedUsage,
    details: &PromptTokensDetails,
    reads: i64,
    writes: i64,
) -> LoggedUsage {
    let other_modalities = details
        .audio_tokens
        .saturating_add(details.image_tokens)
        .saturating_add(details.video_tokens);
    let text_tokens = usage
        .prompt_tokens
        .saturating_sub(reads)
        .saturating_sub(writes)
        .saturating_sub(other_modalities)
        .max(0);
    let cache_creation_token_details = if writes != 0 {
        details.cache_creation_token_details.clone()
    } else {
        None
    };
    LoggedUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        cache_read_input_tokens: reads,
        cache_creation_input_tokens: writes,
        prompt_tokens_details: Some(PromptTokensDetails {
            cached_tokens: reads,
            cache_creation_tokens: writes,
            cache_write_tokens: writes,
            text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            cache_creation_token_details,
        }),
        completion_tokens_details: usage.completion_tokens_details,
    }
}

mod tests;
