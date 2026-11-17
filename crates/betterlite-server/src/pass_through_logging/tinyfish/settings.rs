use rust_decimal::Decimal;

use super::{TINYFISH_DEFAULT_API_BASE, TINYFISH_DEFAULT_STEP_COST};

pub const API_BASE_VARIABLE: &str = "TINYFISH_AGENT_API_BASE";
pub const COST_VARIABLE: &str = "TINYFISH_COST_PER_STEP";

// LiteLLM strips every trailing slash, then promotes a value without a scheme so routing and billing see one host.
#[must_use]
pub fn api_base_from_text(configured: Option<&str>) -> String {
    let raw = match configured {
        Some(value) if !value.is_empty() => value,
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
pub fn cost_per_step_from_text(configured: Option<&str>) -> Decimal {
    let Some(raw) = configured else {
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
