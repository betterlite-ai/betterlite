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

// Python's float() reads an underscore only between two digits, so "1_0" is 10 while "1__0" and "1_" are errors.
fn float_number(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for (position, byte) in bytes.iter().enumerate() {
        if *byte != b'_' {
            continue;
        }
        let before = position.checked_sub(1).and_then(|index| bytes.get(index));
        let after = position.checked_add(1).and_then(|index| bytes.get(index));
        let joins_digits = matches!(
            (before, after),
            (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit()
        );
        if !joins_digits {
            return None;
        }
    }
    Some(text.replace('_', ""))
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
    float_number(trimmed)
        .and_then(|number| number.parse::<Decimal>().ok())
        .unwrap_or_else(|| {
            tracing::warn!(
                "TINYFISH_COST_PER_STEP={raw:?} is not a number; using the default rate 0.016"
            );
            TINYFISH_DEFAULT_STEP_COST
        })
}
