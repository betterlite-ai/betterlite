use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub mod decimal_lexeme;
mod deserialize;
pub mod nested;
pub mod values;

// Each field keeps its long LiteLLM wire name through a serde rename, one line per field.
macro_rules! price_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident { $($field:ident: $wire:literal),* $(,)? }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $(
                #[serde(
                    rename = $wire,
                    deserialize_with = "crate::price_fields::deserialize::optional_price",
                    skip_serializing_if = "Option::is_none"
                )]
                pub $field: Option<rust_decimal::Decimal>,
            )*
        }
    };
}
pub(crate) use price_struct;

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

mod tests;
