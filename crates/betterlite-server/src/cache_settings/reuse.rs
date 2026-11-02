use serde_json::{Map, Value};
use subtle::ConstantTimeEq as _;

use super::target::target_repr;
use super::{CONNECTION_TARGET_FIELDS, REDACTED_VALUE};

// `url` can itself carry a credential (CACHE_SENSITIVE_FIELDS), so it alone is compared in constant time.
#[must_use]
pub fn saved_secret_is_reusable(incoming: &Map<String, Value>, saved: &Map<String, Value>) -> bool {
    for field in CONNECTION_TARGET_FIELDS {
        let saved_value = saved.get(field);
        if is_unpinned(saved_value) {
            continue;
        }
        let incoming_value = incoming.get(field);
        if field == "url" && is_kept_as_is_url(incoming_value) {
            continue;
        }
        let incoming_repr = target_repr(incoming_value.unwrap_or(&Value::Null));
        let saved_repr = target_repr(saved_value.unwrap_or(&Value::Null));
        let targets_match = if field == "url" {
            bool::from(incoming_repr.as_bytes().ct_eq(saved_repr.as_bytes()))
        } else {
            incoming_repr == saved_repr
        };
        if !targets_match {
            return false;
        }
    }
    true
}

// A stored field that is absent or explicitly null carries no pin, same as docs/compatibility.md's absent-equals-null rule.
fn is_unpinned(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(fields)) => fields.is_empty(),
        Some(Value::Bool(_) | Value::Number(_)) => false,
    }
}

fn is_kept_as_is_url(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty() || text == REDACTED_VALUE,
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => false,
    }
}
