use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::{TINYFISH_DEFAULT_API_BASE, TINYFISH_DEFAULT_STEP_COST};

pub(crate) const API_BASE_VARIABLE: &str = "TINYFISH_AGENT_API_BASE";
pub(crate) const COST_VARIABLE: &str = "TINYFISH_COST_PER_STEP";

// LiteLLM strips every trailing slash, then promotes a value without a scheme so routing and billing see one host.
#[must_use]
pub fn api_base_from_environment(environment: &BTreeMap<String, String>) -> String {
    let raw = match environment.get(API_BASE_VARIABLE) {
        Some(value) if !value.is_empty() => value.as_str(),
        _ => TINYFISH_DEFAULT_API_BASE,
    };
    let trimmed = raw.trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    }
}

// A rate that is not a number keeps the run billable at the published default instead of failing the request.
#[must_use]
pub fn cost_per_step_from_environment(environment: &BTreeMap<String, String>) -> Decimal {
    let Some(raw) = environment.get(COST_VARIABLE) else {
        return TINYFISH_DEFAULT_STEP_COST;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return TINYFISH_DEFAULT_STEP_COST;
    }
    match trimmed.parse::<Decimal>() {
        Ok(rate) => rate,
        Err(error) => {
            tracing::warn!(
                %error,
                "TINYFISH_COST_PER_STEP={raw:?} is not a number; using the default rate 0.016"
            );
            TINYFISH_DEFAULT_STEP_COST
        }
    }
}
