use betterlite_models::billed_tokens::custom::{CustomTokenUsage, custom_cost};
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::usage_sum::{Usage, UsageNumber};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::CallPriceInput;
use super::error::CallPriceError;

struct CacheCounts {
    read: i64,
    creation: i64,
    // Set when a top-level or explicit cache counter shows an Anthropic-style usage.
    exclusive_input: bool,
}

// Ports the custom-pricing block of cost_per_token: cache counts are normalized, then a custom token or second price returns at once.
pub fn custom_price(
    input: &CallPriceInput<'_>,
) -> Result<Option<GenericCostResult>, CallPriceError> {
    let counts = cache_counts(input)?;
    let mut prompt_tokens = input.prompt_tokens;
    if counts.exclusive_input {
        prompt_tokens = prompt_tokens
            .checked_add(counts.read)
            .and_then(|total| total.checked_add(counts.creation))
            .ok_or(CallPriceError::CounterOverflow)?;
    }
    let rates = input.custom_cost_per_token.map(|rates| {
        (
            rates.input_cost_per_token,
            rates.output_cost_per_token,
            rates.cache_read_input_token_cost,
            rates.cache_creation_cost,
        )
    });
    let custom = custom_cost(
        CustomTokenUsage {
            prompt_tokens,
            completion_tokens: input.completion_tokens,
            cached_tokens: counts.read,
            cache_creation_tokens: counts.creation,
        },
        rates,
        input.custom_cost_per_second,
        input.response_time_ms,
    )?;
    Ok(
        custom.map(|(prompt_cost, completion_cost)| GenericCostResult {
            prompt_cost,
            completion_cost,
        }),
    )
}

// Details come first, then the usage's top-level counters, then a truthy explicit argument fills a zero count.
fn cache_counts(input: &CallPriceInput<'_>) -> Result<CacheCounts, CallPriceError> {
    let mut counts = CacheCounts {
        read: 0,
        creation: 0,
        exclusive_input: false,
    };
    if let Some(usage) = input.usage {
        if let Some(details) = usage.prompt_tokens_details.as_ref() {
            counts.read = details.cached_tokens.unwrap_or(0);
            counts.creation = match details.cache_write_tokens {
                Some(written) if written != 0 => written,
                _ => details.cache_creation_tokens.unwrap_or(0),
            };
        }
        let top_read = top_level_count(usage, "cache_read_input_tokens")?;
        let top_creation = top_level_count(usage, "cache_creation_input_tokens")?;
        if top_read.is_some() || top_creation.is_some() {
            counts.exclusive_input = true;
            counts.read = top_read.unwrap_or(counts.read);
            counts.creation = top_creation.unwrap_or(counts.creation);
        }
    }
    if counts.read == 0
        && let Some(explicit) = input.cache_read_input_tokens.filter(|count| *count != 0)
    {
        counts.read = explicit;
        counts.exclusive_input = true;
    }
    if counts.creation == 0
        && let Some(explicit) = input
            .cache_creation_input_tokens
            .filter(|count| *count != 0)
    {
        counts.creation = explicit;
        counts.exclusive_input = true;
    }
    Ok(counts)
}

// A top-level counter that is present, even at zero, marks the usage as Anthropic-style.
fn top_level_count(usage: &Usage, field: &'static str) -> Result<Option<i64>, CallPriceError> {
    match usage.numeric_extras.get(field) {
        None => Ok(None),
        Some(UsageNumber::Integer(count)) => Ok(Some(*count)),
        Some(UsageNumber::Decimal(count)) if count.fract() == Decimal::ZERO => count
            .to_i64()
            .map(Some)
            .ok_or(CallPriceError::CounterOverflow),
        Some(UsageNumber::Decimal(_) | UsageNumber::Boolean(_)) => {
            Err(CallPriceError::NonIntegerCount { field })
        }
    }
}
