use rust_decimal::Decimal;

use super::{
    BilledTokenCounts, BilledTokenError, BilledTokenRates, TokenTypeCostBreakdown, breakdown, mul,
};

#[must_use]
pub fn custom_billed_token_rates(
    input: Decimal,
    output: Decimal,
    cache_read: Option<Decimal>,
    cache_creation: Option<Decimal>,
) -> BilledTokenRates {
    let read = cache_read.unwrap_or(input);
    let write = cache_creation.unwrap_or(input);
    BilledTokenRates {
        input_cost_per_token: input,
        output_cost_per_token: output,
        cache_read_input_token_cost: read,
        audio_read: read,
        cache_write: write,
        cache_write_1h: write,
        reasoning: output,
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
pub struct CustomTokenUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens: i64,
}

pub fn custom_cost(
    usage: CustomTokenUsage,
    rates: Option<(Decimal, Decimal, Option<Decimal>, Option<Decimal>)>,
    cost_per_second: Option<Decimal>,
    response_time_ms: Option<Decimal>,
) -> Result<Option<(Decimal, Decimal)>, BilledTokenError> {
    if let Some((input, output, read, write)) = rates {
        let billed = custom_billed_token_rates(input, output, read, write);
        let normal = usage
            .prompt_tokens
            .checked_sub(usage.cached_tokens)
            .and_then(|count| count.checked_sub(usage.cache_creation_tokens))
            .ok_or(BilledTokenError::Overflow)?
            .max(0);
        let input_cost = mul(normal, billed.input_cost_per_token)?
            .checked_add(mul(
                usage.cached_tokens,
                billed.cache_read_input_token_cost,
            )?)
            .and_then(|value| {
                value.checked_add(mul(usage.cache_creation_tokens, billed.cache_write).ok()?)
            })
            .ok_or(BilledTokenError::Overflow)?;
        return Ok(Some((
            input_cost,
            mul(usage.completion_tokens, billed.output_cost_per_token)?,
        )));
    }
    let Some(price) = cost_per_second else {
        return Ok(None);
    };
    let duration = response_time_ms
        .unwrap_or(Decimal::ZERO)
        .checked_div(Decimal::from(1000_u32))
        .ok_or(BilledTokenError::Overflow)?;
    Ok(Some((
        Decimal::ZERO,
        price
            .checked_mul(duration)
            .ok_or(BilledTokenError::Overflow)?,
    )))
}

pub fn custom_breakdown(
    mut counts: BilledTokenCounts,
    rates: BilledTokenRates,
) -> Result<TokenTypeCostBreakdown, BilledTokenError> {
    counts.cache_creation_5m = None;
    counts.cache_creation_1h = None;
    breakdown(counts, Some(rates))
}
