use super::super::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

const SENSITIVE: &[&str] = &[
    "api_key",
    "token",
    "secret",
    "password",
    "cookie",
    "authorization",
    "header",
    "hidden_params",
];
const DROPPED_KEYS: &[&str] = &["function_kwargs", "function_args", "function_name"];

#[must_use]
pub fn sanitize_event_metadata(
    values: &BTreeMap<String, OtelModelValue>,
) -> BTreeMap<String, String> {
    let mut clean = BTreeMap::new();
    for (key, value) in values {
        let lowered = key.to_lowercase();
        if DROPPED_KEYS.contains(&key.as_str())
            || SENSITIVE.iter().any(|needle| lowered.contains(needle))
        {
            continue;
        }
        let text = match value {
            OtelModelValue::String(text) => Some(text.chars().take(1024).collect()),
            OtelModelValue::Number(number) => Some(number.to_string()),
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_) => None,
        };
        if let Some(text_value) = text {
            clean.insert(key.clone(), text_value);
        }
        if clean.len() == 32 {
            break;
        }
    }
    clean
}

#[path = "sanitize_event_metadata/tests.rs"]
mod tests;
