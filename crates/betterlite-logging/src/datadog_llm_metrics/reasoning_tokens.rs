use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::values::{Object, mapping_field};

const REASONING_DETAILS_FIELDS: [&str; 2] = ["completion_tokens_details", "output_tokens_details"];

// The first details mapping holding a numeric reasoning_tokens wins, even when that count is zero
#[must_use]
pub fn reasoning_output_tokens(usage_object: Option<&Object>) -> f64 {
    let Some(usage) = usage_object else {
        return 0.0_f64;
    };
    for field in REASONING_DETAILS_FIELDS {
        let Some(details) = mapping_field(usage, field) else {
            continue;
        };
        if let Some(Json::Number(number)) = details.get("reasoning_tokens")
            && let Some(count) = number.as_f64()
        {
            return count;
        }
    }
    0.0_f64
}
