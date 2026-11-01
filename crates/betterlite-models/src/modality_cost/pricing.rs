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

// Mirrors _get_cost_per_unit: dict.get cannot tell an explicit null from a missing key, so both fall back through the first service-tier suffix found anywhere in the key.
pub fn price_for_key(model: &ModelInfo, key: &str) -> Result<Option<Decimal>, ModalityCostError> {
    match model.get(key) {
        Some(scalar @ (Scalar::Bool(_) | Scalar::Number(_) | Scalar::Text(_))) => {
            Ok(Some(coerce_cost_per_token(Some(scalar))?))
        }
        Some(Scalar::Array(_) | Scalar::Object(_)) => Ok(None),
        Some(Scalar::Null) | None => {
            for suffix in SERVICE_TIER_SUFFIXES {
                if key.contains(suffix) {
                    let base = key.replacen(suffix, "", 1);
                    return price_for_key(model, &base);
                }
            }
            Ok(None)
        }
    }
}

mod tests;
