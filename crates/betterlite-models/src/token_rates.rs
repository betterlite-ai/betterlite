pub mod base_cost;
pub mod batch;
pub mod deployment;
mod rates;
mod reasoning;
mod tests;
mod threshold;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::off_peak::rate::OffPeakRateError;
use crate::off_peak::schema::OffPeakBlock;
use crate::price_fields::decimal_lexeme::DecimalLexemeError;
use crate::tiered_pricing::{Scalar, Tier, TieredPricingError};

// A deployment's raw pricing map, keyed exactly as a provider's YAML model_info entries.
pub type ModelInfo = BTreeMap<String, Scalar>;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Usage {
    pub prompt_tokens: i64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EffectiveRates {
    pub input_rate: Decimal,
    pub output_rate: Decimal,
    pub cache_read_rate: Decimal,
    pub cache_creation_rate: Decimal,
    pub cache_creation_above_1hr_rate: Decimal,
    pub reasoning_rate: Decimal,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BatchCostRates {
    pub input: Option<Decimal>,
    pub output: Option<Decimal>,
    pub cache_read: Option<Decimal>,
    pub cache_creation: Option<Decimal>,
}

// Kept separate from ModelInfo because its Scalar values discard nested arrays and objects.
pub struct ModelPricing<'pricing> {
    pub model: &'pricing ModelInfo,
    pub tiered_pricing: &'pricing [Tier],
    pub off_peak_pricing: Option<&'pricing OffPeakBlock>,
}

pub struct RequestContext<'context> {
    pub service_tier: Option<&'context str>,
    pub provider: Option<&'context str>,
    pub moment: DateTime<Utc>,
    pub database: &'context TimeZoneDatabase,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenRateError {
    #[error("invalid token threshold {key}")]
    InvalidThreshold { key: String },
    #[error("invalid exact rate for {key}")]
    Rate {
        key: String,
        #[source]
        source: DecimalLexemeError,
    },
    #[error("rate multiplication overflow")]
    Overflow,
    #[error(transparent)]
    Tier(#[from] TieredPricingError),
    #[error(transparent)]
    OffPeak(#[from] OffPeakRateError),
}
