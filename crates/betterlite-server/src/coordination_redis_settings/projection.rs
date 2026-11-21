use betterlite_config::sections::settings::JsonValue;

use super::{REDACTED_VALUE, SENSITIVE_FIELDS};
use crate::cache_settings_read::view::carries_value;

// litellm's _redact_credentials(settings or {}): a credential keeps its key and loses its value, a null stays null.
#[must_use]
pub fn redact_credentials(settings: Option<&JsonValue>) -> JsonValue {
    let Some(members) = settings.and_then(JsonValue::as_object) else {
        return JsonValue::Object([].into_iter().collect());
    };
    JsonValue::Object(
        members
            .iter()
            .map(|(key, value)| {
                let redacted = if SENSITIVE_FIELDS.contains(&key.as_str()) && !value.is_null() {
                    JsonValue::String(REDACTED_VALUE.to_owned())
                } else {
                    value.clone()
                };
                (key.clone(), redacted)
            })
            .collect(),
    )
}

// litellm's _redact_all_values: a falsy section yields nothing, otherwise every value becomes the marker.
#[must_use]
pub fn redact_all_values(settings: Option<&JsonValue>) -> JsonValue {
    let Some(members) = settings
        .filter(|value| carries_value(value))
        .and_then(JsonValue::as_object)
    else {
        return JsonValue::Object([].into_iter().collect());
    };
    JsonValue::Object(
        members
            .keys()
            .map(|key| (key.clone(), JsonValue::String(REDACTED_VALUE.to_owned())))
            .collect(),
    )
}

// litellm's _merge_over_saved: only incoming keys survive, and the marker returns the saved value only when that key is saved.
#[must_use]
pub fn merge_over_saved(incoming: &JsonValue, saved: Option<&JsonValue>) -> JsonValue {
    let saved_members = saved.and_then(JsonValue::as_object);
    let Some(incoming_members) = incoming.as_object() else {
        return JsonValue::Object([].into_iter().collect());
    };
    JsonValue::Object(
        incoming_members
            .iter()
            .map(|(key, value)| {
                let merged = match saved_members.and_then(|members| members.get(key)) {
                    Some(saved_value) if value.as_str() == Some(REDACTED_VALUE) => {
                        saved_value.clone()
                    }
                    Some(_) | None => value.clone(),
                };
                (key.clone(), merged)
            })
            .collect(),
    )
}

// litellm's _credential_values + _scrub_credentials: every nonempty credential the caller supplied is replaced wherever it appears.
#[must_use]
pub fn scrub_credentials(message: &str, settings: &JsonValue) -> String {
    let mut scrubbed = message.to_owned();
    let Some(members) = settings.as_object() else {
        return scrubbed;
    };
    for (key, value) in members {
        let Some(secret) = credential_text(key, value) else {
            continue;
        };
        if secret.is_empty() {
            continue;
        }
        scrubbed = scrubbed.replace(&secret, REDACTED_VALUE);
    }
    scrubbed
}

// Python's isinstance(value, (str, int)): a bool is not an int here because the wire type of these fields is a string.
fn credential_text(key: &str, value: &JsonValue) -> Option<String> {
    if !SENSITIVE_FIELDS.contains(&key) {
        return None;
    }
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}
