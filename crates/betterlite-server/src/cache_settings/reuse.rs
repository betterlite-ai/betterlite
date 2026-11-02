use serde_json::{Map, Value};

use super::target::target_repr;
use super::{CONNECTION_TARGET_FIELDS, REDACTED_VALUE};

// A stored secret is reused only when every pinned saved field matches; an omitted or redacted incoming url keeps the stored target.
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
        if incoming_repr != saved_repr {
            return false;
        }
    }
    true
}

fn is_unpinned(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(fields)) => fields.is_empty(),
        Some(_) => false,
    }
}

fn is_kept_as_is_url(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::String(text)) => text.is_empty() || text == REDACTED_VALUE,
        Some(_) => false,
    }
}
