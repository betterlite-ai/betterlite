use rust_decimal::Decimal;

use super::error::ModalityCostError;
use crate::tiered_pricing::{Scalar, coerce_cost_per_token};
use crate::token_rates::ModelInfo;

// token_rates::rates' suffix map is private to B1670's crate, so the tiny table is duplicated here.
const SERVICE_TIER_SUFFIXES: [&str; 4] = ["_ultrafast", "_balanced", "_priority", "_flex"];

fn service_tier_suffix(tier: &str) -> Option<&'static str> {
    match tier.to_ascii_lowercase().as_str() {
        "flex" => Some("_flex"),
        "balanced" => Some("_balanced"),
        "priority" | "fast" => Some("_priority"),
        "ultrafast" => Some("_ultrafast"),
        _ => None,
    }
}

pub fn service_tier_cost_key(base: &str, service_tier: Option<&str>) -> String {
    let Some(suffix) = service_tier.and_then(service_tier_suffix) else {
        return base.to_owned();
    };
    format!("{base}{suffix}")
}

// Mirrors _get_cost_per_unit: a present key, even an explicit null, is used as given; only a wholly missing key falls back through its service-tier suffix.
pub fn price_for_key(model: &ModelInfo, key: &str) -> Result<Option<Decimal>, ModalityCostError> {
    match model.get(key) {
        Some(scalar @ (Scalar::Bool(_) | Scalar::Number(_) | Scalar::Text(_))) => {
            Ok(Some(coerce_cost_per_token(Some(scalar))?))
        }
        Some(Scalar::Array(_) | Scalar::Object(_) | Scalar::Null) => Ok(None),
        None => {
            for suffix in SERVICE_TIER_SUFFIXES {
                if let Some(base) = key.strip_suffix(suffix) {
                    return price_for_key(model, base);
                }
            }
            Ok(None)
        }
    }
}

#[must_use]
pub fn key_has_price(model: &ModelInfo, key: &str) -> bool {
    match model.get(key) {
        None | Some(Scalar::Null) => false,
        Some(
            Scalar::Bool(_)
            | Scalar::Number(_)
            | Scalar::Text(_)
            | Scalar::Array(_)
            | Scalar::Object(_),
        ) => true,
    }
}

mod tests;
