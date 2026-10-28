use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

mod deserialize;
pub mod nested;
pub mod values;
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceFields {
    #[serde(flatten)]
    pub values: values::PricingValues,
    #[serde(rename = "provider_specific_entry")]
    pub provider_specific: nested::ProviderSpecificPricing,
    #[serde(
        rename = "guardrail_cost_per_unit",
        deserialize_with = "serde_with::As::<serde_with::MapPreventDuplicates<serde_with::Same, serde_with::FromInto<deserialize::ExactDecimal>>>::deserialize"
    )]
    pub guardrail_units: BTreeMap<String, Decimal>,
    pub metadata: nested::MetadataPricing,
    pub sample_spec: nested::SamplePricing,
    #[serde(rename = "off_peak_pricing")]
    pub off_peak: Option<nested::OffPeakPricing>,
    #[serde(rename = "search_context_cost_per_query")]
    pub search_context: Option<nested::SearchContextPricing>,
    #[serde(rename = "tiered_pricing")]
    pub tiers: Vec<nested::TierPricing>,
}

#[derive(Debug, thiserror::Error)]
pub enum PriceFieldError {
    #[error("price field {path} must be a JSON number")]
    NotNumber { path: String },
    #[error("price field {path} is not a valid decimal: {value}: {source}")]
    InvalidDecimal {
        path: String,
        value: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("price field {path} must not be null")]
    Null { path: String },
    #[error("price field {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl PriceFields {
    pub fn parse(input: &RawValue) -> Result<Self, PriceFieldError> {
        let mut deserializer = serde_json::Deserializer::from_str(input.get());
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| PriceFieldError::Json {
            path: error.path().to_string(),
            source: error.into_inner(),
        })
    }
}

pub fn parse_decimal_lexeme(lexeme: &str) -> Result<Decimal, rust_decimal::Error> {
    let Some((coefficient, exponent_text)) = lexeme.split_once(['e', 'E']) else {
        return Decimal::from_str_exact(lexeme);
    };
    let decimal = Decimal::from_str_exact(coefficient)?;
    let exponent = exponent_text
        .parse::<i32>()
        .map_err(|_source| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?;
    let scale = i32::try_from(decimal.scale())
        .map_err(|_source| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?
        .checked_sub(exponent)
        .ok_or(rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?;
    if !scale.is_negative() {
        return Decimal::try_from_i128_with_scale(
            decimal.mantissa(),
            u32::try_from(scale)
                .map_err(|_source| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?,
        )
        .map(|value| value.normalize());
    }
    let multiplier = 10_i128
        .checked_pow(scale.unsigned_abs())
        .ok_or(rust_decimal::Error::ExceedsMaximumPossibleValue)?;
    let mantissa = decimal
        .mantissa()
        .checked_mul(multiplier)
        .ok_or(rust_decimal::Error::ExceedsMaximumPossibleValue)?;
    Decimal::try_from_i128_with_scale(mantissa, 0).map(|value| value.normalize())
}

mod tests;
