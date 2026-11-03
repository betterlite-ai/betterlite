use serde_json::Value;
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

pub fn sanitize_event_metadata(values: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    let mut clean = BTreeMap::new();
    for (key, value) in values {
        let lowered = key.to_lowercase();
        if key.starts_with("function_") || SENSITIVE.iter().any(|needle| lowered.contains(needle)) {
            continue;
        }
        let text = match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        };
        if let Some(text) = text {
            clean.insert(key.clone(), text.chars().take(1024).collect());
        }
        if clean.len() == 32 {
            break;
        }
    }
    clean
}

#[path = "sanitize_event_metadata/tests.rs"]
mod tests;
