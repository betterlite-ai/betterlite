use std::fmt::Write as _;

use super::decode::{JsonValue, TypedValue};

#[derive(Clone, Debug, PartialEq)]
pub struct Invalidation {
    pub cache_key: String,
    pub new_value: Option<f64>,
    pub ttl: Option<f64>,
}

#[must_use]
pub fn encode(invalidation: &Invalidation) -> String {
    let mut message = String::from("{\"cache_key\":");
    message.push_str(&quoted(&invalidation.cache_key));
    if let Some(new_value) = invalidation.new_value {
        message.push_str(",\"new_value\":");
        message.push_str(&number_text(new_value));
    }
    if let Some(ttl) = invalidation.ttl {
        message.push_str(",\"ttl\":");
        message.push_str(&number_text(ttl));
    }
    message.push('}');
    message
}

#[must_use]
pub fn read(value: &TypedValue) -> Option<Invalidation> {
    let Some(JsonValue::Text(cache_key)) = value.get("cache_key") else {
        return None;
    };
    Some(Invalidation {
        cache_key: cache_key.clone(),
        new_value: number_or_none(value.get("new_value")),
        ttl: number_or_none(value.get("ttl")),
    })
}

const fn number_or_none(value: Option<&JsonValue>) -> Option<f64> {
    let Some(JsonValue::Number(number)) = value else {
        return None;
    };
    Some(*number)
}

fn number_text(number: f64) -> String {
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number.is_infinite() {
        return if number.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    format!("{number}")
}

fn quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len().saturating_add(2));
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            control if control < ' ' => {
                let code = u32::from(control);
                let _written = write!(quoted, "\\u{code:04x}");
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}
