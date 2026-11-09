use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::TokenCount;
use indexmap::IndexMap;

use super::insert_count;
use crate::cost::usage::{BillingUsage, BillingUsageError};

pub(crate) fn get_usage(usage: &BillingUsage<'_>) -> Result<Option<Usage>, BillingUsageError> {
    match usage {
        BillingUsage::Absent | BillingUsage::Null => Ok(None),
        BillingUsage::DecisionsIr(found) => decisions_usage(
            token_count(&found.input_tokens, "input_tokens")?,
            token_count(&found.output_tokens, "output_tokens")?,
            token_count(&found.cached_tokens, "cached_tokens")?,
            token_count(&found.cache_write_tokens, "cache_write_tokens")?,
        )
        .map(Some),
        BillingUsage::DecisionsOpenAi(found) => decisions_usage(
            token_count(&found.input_tokens, "input_tokens")?,
            token_count(&found.output_tokens, "output_tokens")?,
            token_count(&found.input_tokens_details.cached_tokens, "cached_tokens")?,
            token_count(
                &found.input_tokens_details.cache_write_tokens,
                "cache_write_tokens",
            )?,
        )
        .map(Some),
        BillingUsage::Chat(_)
        | BillingUsage::Response(_)
        | BillingUsage::Transcription(_)
        | BillingUsage::Interactions(_)
        | BillingUsage::Mapping(_)
        | BillingUsage::TypedRecord(_) => Err(BillingUsageError::Projection { kind: "decision" }),
    }
}

// LiteLLM recomputes the total from input and output and never adds Decisions output reasoning.
fn decisions_usage(
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
