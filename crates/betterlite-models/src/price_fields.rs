use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub mod nested;
mod validation;
pub mod values;
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct PriceFields {
    #[serde(flatten)]
    pub values: values::PricingValues,
    #[serde(rename = "provider_specific_entry")]
    pub provider_specific: nested::ProviderSpecificPricing,
    #[serde(
        rename = "guardrail_cost_per_unit",
        deserialize_with = "serde_with::rust::maps_duplicate_key_is_error::deserialize"
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

#[derive(Default, Deserialize)]
#[serde(default)]
struct PriceInput {
    #[serde(flatten)]
    pub values: values::PricingValues,
    #[serde(rename = "provider_specific_entry")]
    pub provider_specific: nested::ProviderSpecificPricing,
    #[serde(
        rename = "guardrail_cost_per_unit",
        deserialize_with = "serde_with::rust::maps_duplicate_key_is_error::deserialize"
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
    #[error("price fields JSON is invalid: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

impl PriceFields {
    pub fn parse(input: &RawValue) -> Result<Self, PriceFieldError> {
        let object: BTreeMap<String, Box<RawValue>> =
            serde_json::from_str(input.get()).map_err(|source| PriceFieldError::Json { source })?;
        validation::validate(&object, "")?;
        let wire: PriceInput =
            serde_json::from_str(input.get()).map_err(|source| PriceFieldError::Json { source })?;
        Ok(Self {
            values: wire.values,
            provider_specific: wire.provider_specific,
            guardrail_units: wire.guardrail_units,
            metadata: wire.metadata,
            sample_spec: wire.sample_spec,
            off_peak: wire.off_peak,
            search_context: wire.search_context,
            tiers: wire.tiers,
        })
    }
}

impl TryFrom<Box<RawValue>> for PriceFields {
    type Error = PriceFieldError;

    fn try_from(input: Box<RawValue>) -> Result<Self, Self::Error> {
        Self::parse(&input)
    }
}

pub(super) fn decimal_at(path: &str, value: &RawValue) -> Result<Decimal, PriceFieldError> {
    let lexeme = value.get();
    if lexeme == "null" {
        return Err(PriceFieldError::Null {
            path: path.to_owned(),
        });
    }
    if lexeme.starts_with('"')
        || lexeme == "true"
        || lexeme == "false"
        || lexeme.starts_with('[')
        || lexeme.starts_with('{')
    {
        return Err(PriceFieldError::NotNumber {
            path: path.to_owned(),
        });
    }
    parse_decimal_lexeme(lexeme).map_err(|source| PriceFieldError::InvalidDecimal {
        path: path.to_owned(),
        value: lexeme.to_owned(),
        source,
    })
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
    if scale >= 0_i32 {
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
