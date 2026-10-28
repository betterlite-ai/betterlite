use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub mod nested;
pub mod values;
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceFields {
    #[serde(flatten)]
    pub values: values::PricingValues,
    #[serde(rename = "provider_specific_entry")]
    pub provider_specific: nested::ProviderSpecificPricing,
    #[serde(rename = "guardrail_cost_per_unit")]
    #[serde(deserialize_with = "decimal_map")]
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
        for (name, value) in object {
            if matches!(
                name.as_str(),
                "input_cost_per_token"
                    | "output_cost_per_token"
                    | "input_cost_per_audio_token"
                    | "output_cost_per_audio_token"
                    | "input_cost_per_image"
                    | "output_cost_per_image"
                    | "input_cost_per_video_token"
                    | "output_cost_per_video_token"
                    | "input_cost_per_character"
                    | "output_cost_per_character"
                    | "input_cost_per_pixel"
                    | "output_cost_per_pixel"
                    | "input_cost_per_query"
                    | "input_cost_per_request"
                    | "input_cost_per_second"
                    | "output_cost_per_second"
                    | "cost_per_second"
                    | "citation_cost_per_token"
                    | "code_interpreter_cost_per_session"
                    | "google_maps_grounding_cost_per_query"
                    | "ocr_cost_per_credit"
                    | "ocr_cost_per_page"
                    | "regional_endpoint_uplift_multiplier"
                    | "regional_processing_uplift_multiplier_eu"
                    | "regional_processing_uplift_multiplier_us"
            ) {
                decimal_at(&name, value.as_ref())?;
            }
        }
        serde_json::from_str(input.get()).map_err(|source| PriceFieldError::Json { source })
    }
}

pub(crate) fn decimal_option<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let lexeme = match &value {
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::String(_) => "\"string\"".to_owned(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Array(_) => "[]".to_owned(),
        serde_json::Value::Object(_) => "{}".to_owned(),
    };
    let raw = RawValue::from_string(lexeme).map_err(serde::de::Error::custom)?;
    decimal_at("price", raw.as_ref())
        .map(Some)
        .map_err(serde::de::Error::custom)
}

fn decimal_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut result = BTreeMap::new();
    for (name, value) in values {
        let lexeme = match value {
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Null => "null".to_owned(),
            serde_json::Value::String(_) => "\"string\"".to_owned(),
            serde_json::Value::Bool(value) => value.to_string(),
            serde_json::Value::Array(_) => "[]".to_owned(),
            serde_json::Value::Object(_) => "{}".to_owned(),
        };
        let raw = RawValue::from_string(lexeme).map_err(serde::de::Error::custom)?;
        let value = decimal_at(&format!("guardrail_cost_per_unit.{name}"), raw.as_ref())
            .map_err(serde::de::Error::custom)?;
        result.insert(name, value);
    }
    Ok(result)
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

fn parse_decimal_lexeme(lexeme: &str) -> Result<Decimal, rust_decimal::Error> {
    let Some((mantissa, exponent)) = lexeme.split_once(['e', 'E']) else {
        return Decimal::from_str_exact(lexeme);
    };
    let decimal = Decimal::from_str_exact(mantissa)?;
    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?;
    let scale = i32::try_from(decimal.scale())
        .map_err(|_| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?
        - exponent;
    if scale >= 0 {
        return Decimal::try_from_i128_with_scale(
            decimal.mantissa(),
            u32::try_from(scale)
                .map_err(|_| rust_decimal::Error::ScaleExceedsMaximumPrecision(29))?,
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

#[cfg(test)]
mod tests;
