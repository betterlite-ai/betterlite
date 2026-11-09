use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::TokenCount;
use betterlite_models::decisions::openai::DecisionUsage;
use betterlite_models::decisions::systemone::DecisionsUsage;
use indexmap::IndexMap;

use super::insert_count;
use crate::cost::usage::BillingUsageError;

pub(crate) fn from_ir_usage(usage: &DecisionsUsage) -> Result<Usage, BillingUsageError> {
    let prompt_tokens = token_count(&usage.input_tokens, "input_tokens")?;
    let completion_tokens = token_count(&usage.output_tokens, "output_tokens")?;
    let cached_tokens = token_count(&usage.cached_tokens, "cached_tokens")?;
    let cache_write_tokens = token_count(&usage.cache_write_tokens, "cache_write_tokens")?;
    decisions_response(
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        cache_write_tokens,
    )
}

pub(crate) fn from_openai_usage(usage: &DecisionUsage) -> Result<Usage, BillingUsageError> {
    let prompt_tokens = token_count(&usage.input_tokens, "input_tokens")?;
    let completion_tokens = token_count(&usage.output_tokens, "output_tokens")?;
    let cached_tokens = token_count(&usage.input_tokens_details.cached_tokens, "cached_tokens")?;
    let cache_write_tokens = token_count(
        &usage.input_tokens_details.cache_write_tokens,
        "cache_write_tokens",
    )?;
    decisions_response(
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        cache_write_tokens,
    )
}

// LiteLLM recomputes the total from input and output and never adds Decisions output reasoning.
fn decisions_response(
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
    cache_write_tokens: i64,
) -> Result<Usage, BillingUsageError> {
    let total_tokens =
        prompt_tokens
            .checked_add(completion_tokens)
            .ok_or(BillingUsageError::TokenOverflow {
                field: "total_tokens",
            })?;
    let mut details = IndexMap::new();
    insert_count(&mut details, "cached_tokens", Some(cached_tokens));
    insert_count(&mut details, "cache_write_tokens", Some(cache_write_tokens));
    insert_count(
        &mut details,
        "cache_creation_tokens",
        Some(cache_write_tokens),
    );
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: Some(details),
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    })
}

fn token_count(count: &TokenCount, field: &'static str) -> Result<i64, BillingUsageError> {
    count
        .0
        .as_i64()
        .ok_or(BillingUsageError::TokenCount { field })
}
