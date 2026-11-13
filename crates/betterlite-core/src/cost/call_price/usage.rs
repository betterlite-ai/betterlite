use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageNumber};

use super::CallPriceInput;
use super::error::CallPriceError;

// The B1668 projection of the usage the call is priced from: the supplied block, or the one built from the explicit counters.
pub struct UsageView {
    pub cost: CostUsage,
    pub extras: PromptExtras,
}

pub fn usage_view(input: &CallPriceInput<'_>) -> Result<UsageView, CallPriceError> {
    let built;
    let usage = if let Some(supplied) = input.usage {
        supplied
    } else {
        built = construct(input)?;
        &built
    };
    Ok(UsageView {
        cost: CostUsage::from(usage),
        extras: prompt_extras(usage),
    })
}

fn prompt_extras(usage: &Usage) -> PromptExtras {
    let Some(details) = usage.prompt_tokens_details.as_ref() else {
        return PromptExtras::default();
    };
    PromptExtras {
        cache_write_tokens: details.cache_write_tokens,
        cache_creation_tokens: details.cache_creation_tokens,
        cache_creation_token_details: None,
        cached_tokens_details: details.cached_tokens_details.as_ref().map(|cached| {
            CachedTokenCounters {
                text_tokens: cached.text_tokens,
                audio_tokens: cached.audio_tokens,
                image_tokens: cached.image_tokens,
            }
        }),
        character_count: details.character_count,
        image_count: details.image_count,
        video_length_seconds: details.video_length_seconds,
        audio_length_seconds: details.audio_length_seconds,
        query_count: details.query_count,
    }
}

// LiteLLM's Usage(...) keeps an integer cache counter as a detail and as a private and a public attribute.
fn construct(input: &CallPriceInput<'_>) -> Result<Usage, CallPriceError> {
    let total_tokens = input
        .prompt_tokens
        .checked_add(input.completion_tokens)
        .ok_or(CallPriceError::CounterOverflow)?;
    let read = input.cache_read_input_tokens;
    let creation = input.cache_creation_input_tokens;
    let mut numeric_extras = BTreeMap::new();
    for (name, counter) in [
        ("cache_read_input_tokens", read),
        ("cache_creation_input_tokens", creation),
    ] {
        if let Some(count) = counter {
            numeric_extras.insert(format!("_{name}"), UsageNumber::Integer(count));
            numeric_extras.insert(name.to_owned(), UsageNumber::Integer(count));
        }
    }
    let has_cache = read.is_some() || creation.is_some();
    Ok(Usage {
        prompt_tokens: input.prompt_tokens,
        completion_tokens: input.completion_tokens,
        total_tokens,
        prompt_tokens_details: has_cache.then(|| cache_details(read, None, creation)),
        completion_tokens_details: None,
        numeric_extras,
    })
}

// Names every field so a new counter in the projection cannot go unpriced unnoticed.
pub(super) const fn cache_details(
    cached: Option<i64>,
    creation: Option<i64>,
    write: Option<i64>,
) -> PromptTokensDetails {
    PromptTokensDetails {
        text_tokens: None,
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: write,
        cache_creation_tokens: creation,
        cached_tokens: cached,
        cached_tokens_details: None,
    }
}
