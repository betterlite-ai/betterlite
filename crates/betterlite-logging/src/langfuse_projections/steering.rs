use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::text::optional_str;

// litellm.secret_managers.main.str_to_bool recognizes only these two words after strip and lower
#[must_use]
pub fn recognized_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

// Python's bool() over a JSON value, which _as_steering_flag falls back to
#[must_use]
pub fn truthiness(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => {
            Decimal::from_str(&number.to_string()).is_ok_and(|amount| !amount.is_zero())
        }
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub fn as_steering_flag(value: &Json) -> bool {
    match value {
        Json::Text(text) => recognized_bool(text).unwrap_or(!text.is_empty()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            truthiness(value)
        }
    }
}

#[must_use]
pub fn as_steering_key_sequence(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::Text(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| optional_str(Some(item)))
            .collect(),
        Some(Json::Object(fields)) => fields.keys().cloned().collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_)) | None => Vec::new(),
    }
}

#[must_use]
pub fn trace_public_flag(value: Option<&Json>) -> Option<bool> {
    match value {
        Some(Json::Null) | None => None,
        Some(flag) => Some(as_steering_flag(flag)),
    }
}
