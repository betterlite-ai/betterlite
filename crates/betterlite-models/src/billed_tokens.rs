use rust_decimal::Decimal;

use crate::price_lookup::{PriceRecord, PricedModel};
use crate::token_rates::TokenRateError;

pub mod catalog;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BilledTokenRates {
    pub input_cost_per_token: Decimal,
    pub output_cost_per_token: Decimal,
    pub cache_read_input_token_cost: Decimal,
    pub audio_read: Decimal,
    pub cache_write: Decimal,
    pub cache_write_1h: Decimal,
    pub reasoning: Decimal,
}

impl BilledTokenRates {
    pub fn scaled(self, multiplier: Decimal) -> Result<Self, BilledTokenError> {
        if multiplier == Decimal::ONE {
            return Ok(self);
        }
        Ok(Self {
            input_cost_per_token: scale(self.input_cost_per_token, multiplier)?,
            output_cost_per_token: scale(self.output_cost_per_token, multiplier)?,
            cache_read_input_token_cost: scale(self.cache_read_input_token_cost, multiplier)?,
            audio_read: scale(self.audio_read, multiplier)?,
            cache_write: scale(self.cache_write, multiplier)?,
            cache_write_1h: scale(self.cache_write_1h, multiplier)?,
            reasoning: scale(self.reasoning, multiplier)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenTypeCostBreakdown {
    pub reasoning_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub rates: Option<BilledTokenRates>,
}

#[derive(Debug, thiserror::Error)]
pub enum BilledTokenError {
    #[error("billed token arithmetic overflowed")]
    Overflow,
    #[error(transparent)]
    Rate(#[from] TokenRateError),
    #[error(transparent)]
    Modality(#[from] crate::modality_cost::error::ModalityCostError),
    #[error(transparent)]
    TokenCost(#[from] crate::token_cost::error::TokenCostError),
}

pub struct BilledTokenCounts {
    pub reasoning: i64,
    pub cache_read: i64,
    pub cached_audio: i64,
    pub cache_creation: i64,
    pub cache_creation_5m: Option<i64>,
    pub cache_creation_1h: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BilledTokenUsage {
    pub detail_reasoning: Option<i64>,
    pub top_level_reasoning: Option<i64>,
    pub detail_cache_read: Option<i64>,
    pub top_level_cache_read: Option<i64>,
    pub detail_cache_creation: Option<i64>,
    pub top_level_cache_creation: Option<i64>,
    pub detail_cached_audio: Option<i64>,
    pub detail_cache_creation_5m: Option<i64>,
    pub detail_cache_creation_1h: Option<i64>,
}

pub fn project_billed_token_counts(usage: BilledTokenUsage) -> BilledTokenCounts {
    BilledTokenCounts {
        reasoning: usage
            .detail_reasoning
            .filter(|count| *count != 0)
            .unwrap_or_else(|| {
                usage
                    .top_level_reasoning
                    .filter(|count| *count > 0)
                    .unwrap_or(0)
            }),
        cache_read: usage
            .detail_cache_read
            .filter(|count| *count != 0)
            .unwrap_or_else(|| {
                usage
                    .top_level_cache_read
                    .filter(|count| *count > 0)
                    .unwrap_or(0)
            }),
        cached_audio: usage.detail_cached_audio.unwrap_or(0),
        cache_creation: usage
            .detail_cache_creation
            .filter(|count| *count != 0)
            .unwrap_or_else(|| {
                usage
                    .top_level_cache_creation
                    .filter(|count| *count > 0)
                    .unwrap_or(0)
            }),
        cache_creation_5m: usage.detail_cache_creation_5m,
        cache_creation_1h: usage.detail_cache_creation_1h,
    }
}

pub struct BilledTokenRequest<'request> {
    pub model: &'request str,
    pub provider: Option<&'request str>,
    pub pricing: Option<&'request PricedModel>,
    pub service_tier: Option<&'request str>,
    pub residency: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
    pub inference_geo: Option<&'request str>,
    pub provider_specific_entry: Option<&'request PriceRecord>,
    pub moment: chrono::DateTime<chrono::Utc>,
    pub prompt_tokens: i64,
}

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

pub fn custom_cost(
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
    cache_creation_tokens: i64,
    rates: Option<(Decimal, Decimal, Option<Decimal>, Option<Decimal>)>,
    cost_per_second: Option<Decimal>,
    response_time_ms: Option<Decimal>,
) -> Result<Option<(Decimal, Decimal)>, BilledTokenError> {
    if let Some((input, output, read, write)) = rates {
        let normal = prompt_tokens
            .checked_sub(cached_tokens)
            .and_then(|count| count.checked_sub(cache_creation_tokens))
            .ok_or(BilledTokenError::Overflow)?
            .max(0);
        let input_cost = mul(normal, input)?
            .checked_add(mul(cached_tokens, read.unwrap_or(input))?)
            .and_then(|value| {
                value.checked_add(mul(cache_creation_tokens, write.unwrap_or(input)).ok()?)
            })
            .ok_or(BilledTokenError::Overflow)?;
        return Ok(Some((input_cost, mul(completion_tokens, output)?)));
    }
    let Some(price) = cost_per_second else {
        return Ok(None);
    };
    let duration = response_time_ms
        .unwrap_or(Decimal::ZERO)
        .checked_div(Decimal::from(1000))
        .ok_or(BilledTokenError::Overflow)?;
    Ok(Some((
        Decimal::ZERO,
        price
            .checked_mul(duration)
            .ok_or(BilledTokenError::Overflow)?,
    )))
}

pub fn breakdown(
    counts: BilledTokenCounts,
    rates: Option<BilledTokenRates>,
) -> Result<TokenTypeCostBreakdown, BilledTokenError> {
    let Some(rates) = rates else {
        return Ok(TokenTypeCostBreakdown {
            reasoning_cost: Decimal::ZERO,
            cache_read_cost: Decimal::ZERO,
            cache_creation_cost: Decimal::ZERO,
            rates: None,
        });
    };
    let uncached_read = counts
        .cache_read
        .checked_sub(counts.cached_audio)
        .ok_or(BilledTokenError::Overflow)?;
    let read = mul(uncached_read, rates.cache_read_input_token_cost)?
        .checked_add(mul(counts.cached_audio, rates.audio_read)?)
        .ok_or(BilledTokenError::Overflow)?;
    let creation =
        if let (Some(five), Some(one)) = (counts.cache_creation_5m, counts.cache_creation_1h) {
            mul(five, rates.cache_write)?
                .checked_add(mul(one, rates.cache_write_1h)?)
                .ok_or(BilledTokenError::Overflow)?
        } else {
            mul(counts.cache_creation, rates.cache_write)?
        };
    Ok(TokenTypeCostBreakdown {
        reasoning_cost: mul(counts.reasoning, rates.reasoning)?,
        cache_read_cost: read,
        cache_creation_cost: creation,
        rates: Some(rates),
    })
}

pub fn caching_savings(
    input_rate: Decimal,
    cache_read_rate: Option<Decimal>,
    cache_creation_rate: Option<Decimal>,
    cache_creation_above_1hr_rate: Option<Decimal>,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    cache_creation_5m_tokens: Option<i64>,
    cache_creation_1h_tokens: Option<i64>,
    multiplier: Decimal,
) -> Result<Decimal, BilledTokenError> {
    let write = cache_creation_rate
        .filter(|rate| !rate.is_zero())
        .unwrap_or(input_rate);
    let write_1h = cache_creation_above_1hr_rate
        .filter(|rate| !rate.is_zero())
        .unwrap_or(write);
    let read = cache_read_rate.unwrap_or(input_rate);
    let read_discount = mul(
        cache_read_tokens.max(0),
        (input_rate - read).max(Decimal::ZERO),
    )?;
    let five_minute = cache_creation_5m_tokens
        .unwrap_or(cache_creation_tokens)
        .max(0);
    let one_hour = cache_creation_1h_tokens.unwrap_or(0).max(0);
    let write_premium = mul(five_minute, write - input_rate)?
        .checked_add(mul(one_hour, write_1h - input_rate)?)
        .ok_or(BilledTokenError::Overflow)?;
    read_discount
        .checked_sub(write_premium)
        .and_then(|value| value.checked_mul(multiplier))
        .ok_or(BilledTokenError::Overflow)
}

fn scale(value: Decimal, multiplier: Decimal) -> Result<Decimal, BilledTokenError> {
    value
        .checked_mul(multiplier)
        .ok_or(BilledTokenError::Overflow)
}

fn mul(count: i64, rate: Decimal) -> Result<Decimal, BilledTokenError> {
    Decimal::from(count)
        .checked_mul(rate)
        .ok_or(BilledTokenError::Overflow)
}

mod tests;
