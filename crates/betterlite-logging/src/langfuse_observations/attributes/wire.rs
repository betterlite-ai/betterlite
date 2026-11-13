use betterlite_models::decisions::Json;

use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

// litellm's _serialize: a string travels as it is, everything else as its JSON text, and null is absent
#[must_use]
pub(crate) fn serialized(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None).unwrap_or_default()
        }
    }
}

// litellm's _string_or_none; betterlite writes numbers and containers as JSON text rather than Python's repr
#[must_use]
pub(crate) fn string_or_none(value: Option<&Json>) -> Option<String> {
    value
        .filter(|candidate| !matches!(candidate, Json::Null))
        .map(serialized)
}
