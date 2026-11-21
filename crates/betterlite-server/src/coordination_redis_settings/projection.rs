use betterlite_config::sections::settings::JsonValue;

use super::{REDACTED_VALUE, SENSITIVE_FIELDS};
use crate::cache_settings_read::view::carries_value;

// Which values a read masks: litellm redacts the three credential fields on GET and every value in an audit row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redaction {
    Credentials,
    Audit,
}

// litellm's _redact_credentials(settings or {}) and _redact_all_values: one projection, with the audit mode masking every value.
#[must_use]
pub fn redact(settings: Option<&JsonValue>, mode: Redaction) -> JsonValue {
    let selected = match mode {
        Redaction::Credentials => settings.and_then(JsonValue::as_object),
        Redaction::Audit => settings
            .filter(|value| carries_value(value))
            .and_then(JsonValue::as_object),
    };
    let Some(members) = selected else {
        return JsonValue::Object(std::iter::empty::<(String, JsonValue)>().collect());
    };
    JsonValue::Object(
        members
            .iter()
            .map(|(key, value)| {
                let redacted = match mode {
                    Redaction::Credentials => {
                        if SENSITIVE_FIELDS.contains(&key.as_str()) && !value.is_null() {
                            JsonValue::String(REDACTED_VALUE.to_owned())
                        } else {
                            value.clone()
                        }
                    }
                    Redaction::Audit => JsonValue::String(REDACTED_VALUE.to_owned()),
                };
                (key.clone(), redacted)
            })
            .collect(),
    )
}

// litellm's _merge_over_saved: only incoming keys survive, and the marker returns the saved value only when that key is saved.
#[must_use]
pub fn merge_over_saved(incoming: &JsonValue, saved: Option<&JsonValue>) -> JsonValue {
    let saved_members = saved.and_then(JsonValue::as_object);
    let Some(incoming_members) = incoming.as_object() else {
        return JsonValue::Object(std::iter::empty::<(String, JsonValue)>().collect());
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
