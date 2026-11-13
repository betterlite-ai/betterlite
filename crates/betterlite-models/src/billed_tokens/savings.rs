use crate::modality_cost::cache_write::calculate_cache_writing_cost;
use crate::price_lookup::PricedModel;
use crate::token_cost::regional::regional_uplift_multiplier;
use crate::token_cost::regional::vertex_endpoint_uplift;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::{BilledTokenError, mul};

fn write_premium(
    rates: super::BilledTokenRates,
    counts: super::BilledTokenCounts,
) -> Result<Decimal, BilledTokenError> {
    let write = if rates.cache_write.is_zero() {
        rates.input_cost_per_token
    } else {
        rates.cache_write
    };
    let write_1h = if rates.cache_write_1h.is_zero() {
        write
    } else {
        rates.cache_write_1h
    };
    let details = counts.cache_creation_details().map(|mut details| {
        details.ephemeral_5m_input_tokens =
            details.ephemeral_5m_input_tokens.map(|count| count.max(0));
        details.ephemeral_1h_input_tokens =
            details.ephemeral_1h_input_tokens.map(|count| count.max(0));
        details
    });
    let premium = calculate_cache_writing_cost(
        counts.cache_creation.max(0),
        details,
        write
            .checked_sub(rates.input_cost_per_token)
            .ok_or(BilledTokenError::Overflow)?,
        write_1h
            .checked_sub(rates.input_cost_per_token)
            .ok_or(BilledTokenError::Overflow)?,
    )?;
    Ok(premium)
}

pub fn caching_savings(
    rates: super::BilledTokenRates,
    counts: super::BilledTokenCounts,
    multiplier: Decimal,
) -> Result<Decimal, BilledTokenError> {
    let discount = rates
        .input_cost_per_token
        .checked_sub(rates.cache_read_input_token_cost)
        .ok_or(BilledTokenError::Overflow)?
        .max(Decimal::ZERO);
    let premium = write_premium(rates, counts)?;
    mul(counts.cache_read.max(0), discount)?
        .checked_sub(premium)
        .and_then(|value| value.checked_mul(multiplier))
        .ok_or(BilledTokenError::Overflow)
}

pub fn model_caching_savings(
    request: &super::BilledTokenRequest<'_>,
    priced: &PricedModel,
    mut usage: super::BilledTokenUsage,
    database: &TimeZoneDatabase,
) -> Result<Decimal, BilledTokenError> {
    usage.top_level_cache_read = None;
    usage.top_level_cache_creation = None;
    let counts = super::project_billed_token_counts(usage);
    let defaulted = super::catalog::default_pricing(priced, request.provider)?;
    let rates = super::catalog::base_rates(request, &defaulted, database)?;
    let multiplier = regional_uplift_multiplier(&priced.record, request.residency)
        .checked_mul(vertex_endpoint_uplift(
            &priced.record,
            request.vertex_location,
        ))
        .ok_or(BilledTokenError::Overflow)?;
    caching_savings(
        super::BilledTokenRates {
            input_cost_per_token: rates.input_rate,
            output_cost_per_token: rates.output_rate,
            cache_read_input_token_cost: rates.cache_read_rate,
            audio_read: rates.cache_read_rate,
            cache_write: rates.cache_creation_rate,
            cache_write_1h: rates.cache_creation_above_1hr_rate,
            reasoning: rates.reasoning_rate,
        },
        counts,
        multiplier,
    )
}
