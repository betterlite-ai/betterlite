use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;

use super::BillingResponse;

const PRICED_SERVICE_TIERS: [&str; 5] = ["balanced", "fast", "flex", "priority", "ultrafast"];
const BASE_PRICING_SERVICE_TIERS: [&str; 2] = ["default", "standard"];

#[must_use]
pub fn served_service_tier(
    response: &BillingResponse<'_>,
    usage: Option<&Usage>,
) -> Option<String> {
    if let Some(tier) = response.service_tier {
        return Some(tier.to_owned());
    }
    if let Some(tier) = usage.and_then(usage_service_tier) {
        return Some(tier.to_owned());
    }
    let hidden = response.hidden_params?;
    let provider_specific = hidden.get("provider_specific_fields");
    let traffic_type = match provider_specific {
        Some(Json::Object(fields)) => fields.get("traffic_type"),
        None | Some(Json::Null) => None,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => return None,
    };
    let Json::Text(traffic) = traffic_type? else {
        return None;
    };
    if traffic.is_empty() {
        return None;
    }
    Some(traffic_type_tier(traffic).to_owned())
}

fn usage_service_tier(usage: &Usage) -> Option<&str> {
    match usage.extras.get("service_tier") {
        Some(Json::Text(tier)) => Some(tier),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// ON_DEMAND and an unknown traffic type both fall back to standard pricing.
fn traffic_type_tier(traffic_type: &str) -> &'static str {
    match traffic_type.to_uppercase().as_str() {
        "ON_DEMAND_PRIORITY" => "priority",
        "FLEX" | "BATCH" | "ON_DEMAND_FLEX" => "flex",
        _ => "default",
    }
}

#[must_use]
pub const fn normalize_service_tier(service_tier: Option<&str>) -> Option<&str> {
    match service_tier {
        Some(tier) if !tier.eq_ignore_ascii_case("auto") => Some(tier),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn resolve_billable_service_tier(
    requested: Option<&str>,
    served: Option<&str>,
) -> Option<String> {
    let Some(served_tier) = served else {
        return normalize_service_tier(requested).map(str::to_owned);
    };
    let lower = served_tier.to_lowercase();
    if PRICED_SERVICE_TIERS.contains(&lower.as_str()) {
        return Some(lower);
    }
    if BASE_PRICING_SERVICE_TIERS.contains(&lower.as_str()) {
        return None;
    }
    normalize_service_tier(requested).map(str::to_owned)
}
