use rust_decimal::Decimal;

use super::error::TokenCostError;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::PriceRecord;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

const VALID_DATA_RESIDENCIES: [&str; 2] = ["us", "eu"];

// Mirrors float(): a bool converts, a number or numeric string converts, anything else warns and defaults to one.
fn parse_uplift_multiplier(value: Option<&Scalar>, key: &str) -> Decimal {
    match value {
        None | Some(Scalar::Null) => Decimal::ONE,
        Some(Scalar::Bool(flag)) => Decimal::from(u8::from(*flag)),
        Some(Scalar::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).unwrap_or_else(|_| warn_invalid(key))
        }
        Some(Scalar::Text(text)) => {
            parse_decimal_lexeme(text.trim()).unwrap_or_else(|_| warn_invalid(key))
        }
        Some(Scalar::Array(_) | Scalar::Object(_)) => warn_invalid(key),
    }
}

fn warn_invalid(key: &str) -> Decimal {
    tracing::warn!(
        key,
        "invalid regional uplift multiplier for model; defaulting to 1.0"
    );
    Decimal::ONE
}

// OpenAI's regional-processing uplift: only lowercased "us"/"eu" are recognized, with no trimming.
#[must_use]
pub fn regional_uplift_multiplier(model: &ModelInfo, data_residency: Option<&str>) -> Decimal {
    let Some(residency) = data_residency else {
        return Decimal::ONE;
    };
    let lowered = residency.to_lowercase();
    if !VALID_DATA_RESIDENCIES.contains(&lowered.as_str()) {
        return Decimal::ONE;
    }
    let key = format!("regional_processing_uplift_multiplier_{lowered}");
    parse_uplift_multiplier(model.get(&key), &key)
}

// Vertex AI's non-global endpoint uplift: absent or case-insensitive "global" means no uplift.
#[must_use]
pub fn vertex_endpoint_uplift(model: &ModelInfo, vertex_location: Option<&str>) -> Decimal {
    let is_global = vertex_location.is_none_or(|location| location.eq_ignore_ascii_case("global"));
    if is_global {
        return Decimal::ONE;
    }
    parse_uplift_multiplier(
        model.get("regional_endpoint_uplift_multiplier"),
        "regional_endpoint_uplift_multiplier",
    )
}

// Unlike the other two uplifts, an invalid present geo price propagates as an error rather than defaulting.
pub fn provider_geo_multiplier(
    provider_specific_entry: Option<&PriceRecord>,
    inference_geo: Option<&str>,
) -> Result<Decimal, TokenCostError> {
    let Some(geo) = inference_geo else {
        return Ok(Decimal::ONE);
    };
    let lowered = geo.to_lowercase();
    if lowered == "global" || lowered == "not_available" {
        return Ok(Decimal::ONE);
    }
    let Some(entry) = provider_specific_entry else {
        return Ok(Decimal::ONE);
    };
    match entry.get(&lowered) {
        None => Ok(Decimal::ONE),
        Some(Scalar::Bool(flag)) => Ok(Decimal::from(u8::from(*flag))),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_err(|_error| TokenCostError::InvalidGeoMultiplier),
        Some(Scalar::Text(text)) => {
            parse_decimal_lexeme(text.trim()).map_err(|_error| TokenCostError::InvalidGeoMultiplier)
        }
        Some(Scalar::Null | Scalar::Array(_) | Scalar::Object(_)) => {
            Err(TokenCostError::InvalidGeoMultiplier)
        }
    }
}

mod tests;
