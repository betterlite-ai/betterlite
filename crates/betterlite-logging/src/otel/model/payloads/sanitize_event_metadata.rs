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

#[must_use]
pub fn sanitize_event_metadata(
    values: &BTreeMap<String, OtelModelValue>,
) -> BTreeMap<String, String> {
    let mut clean = BTreeMap::new();
    for (key, value) in values {
        let lowered = key.to_lowercase();
        if key.starts_with("function_") || SENSITIVE.iter().any(|needle| lowered.contains(needle)) {
            continue;
        }
        let text = match value {
            OtelModelValue::String(text) => Some(text.clone()),
            OtelModelValue::Number(number) => Some(number.to_string()),
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_) => None,
        };
        if let Some(text_value) = text {
            clean.insert(key.clone(), text_value.chars().take(1024).collect());
        }
        if clean.len() == 32 {
            break;
        }
    }
    clean
}

#[path = "sanitize_event_metadata/tests.rs"]
mod tests;
