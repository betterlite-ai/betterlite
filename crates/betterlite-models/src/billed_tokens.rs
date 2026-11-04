use crate::cost_usage::modality_projection::CacheCreationTokenDetails;
use crate::modality_cost::cache_write::calculate_cache_writing_cost;
use crate::modality_cost::error::ModalityCostError;
use crate::token_cost::error::TokenCostError;
use rust_decimal::Decimal;

use crate::price_lookup::{PriceRecord, PricedModel};
use crate::token_rates::TokenRateError;

pub mod catalog;
pub mod custom;
pub mod savings;

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
    Modality(#[from] ModalityCostError),
    #[error(transparent)]
    TokenCost(#[from] TokenCostError),
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct BilledTokenCounts {
    pub reasoning: i64,
    pub cache_read: i64,
    pub cached_audio: i64,
    pub cache_creation: i64,
    pub cache_creation_5m: Option<i64>,
    pub cache_creation_1h: Option<i64>,
}

impl BilledTokenCounts {
    #[must_use]
    pub const fn cache_creation_details(self) -> Option<CacheCreationTokenDetails> {
        if self.cache_creation_5m.is_some() || self.cache_creation_1h.is_some() {
            Some(CacheCreationTokenDetails {
                ephemeral_5m_input_tokens: self.cache_creation_5m,
                ephemeral_1h_input_tokens: self.cache_creation_1h,
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize)]
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

#[must_use]
pub fn project_billed_token_counts(usage: BilledTokenUsage) -> BilledTokenCounts {
    BilledTokenCounts {
        reasoning: detail_or_positive(usage.detail_reasoning, usage.top_level_reasoning),
        cache_read: detail_or_positive(usage.detail_cache_read, usage.top_level_cache_read),
        cached_audio: usage
            .detail_cached_audio
            .unwrap_or(0)
            .min(usage.detail_cache_read.unwrap_or(0)),
        cache_creation: detail_or_positive(
            usage.detail_cache_creation,
            usage.top_level_cache_creation,
        ),
        cache_creation_5m: usage.detail_cache_creation_5m,
        cache_creation_1h: usage.detail_cache_creation_1h,
    }
}

fn detail_or_positive(detail: Option<i64>, top_level: Option<i64>) -> i64 {
    match detail {
        Some(count) if count != 0 => count,
        _ => top_level.filter(|count| *count > 0).unwrap_or(0),
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

pub fn breakdown(
    counts: BilledTokenCounts,
    resolved_rates: Option<BilledTokenRates>,
) -> Result<TokenTypeCostBreakdown, BilledTokenError> {
    let Some(rates) = resolved_rates else {
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
    let creation = calculate_cache_writing_cost(
        counts.cache_creation,
        counts.cache_creation_details(),
        rates.cache_write,
        rates.cache_write_1h,
    )?;
    Ok(TokenTypeCostBreakdown {
        reasoning_cost: mul(counts.reasoning, rates.reasoning)?,
        cache_read_cost: read,
        cache_creation_cost: creation,
        rates: Some(rates),
    })
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
