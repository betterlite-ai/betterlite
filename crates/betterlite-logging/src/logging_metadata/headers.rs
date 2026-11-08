use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::fields::payload::records::{AdditionalHeaders, StandardLoggingHiddenParams};
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::keys::HIDDEN_PARAM_KEYS;

// StandardLoggingAdditionalHeaders fields and the header spelling each one reads
const TYPED_HEADERS: [(&str, &str); 6] = [
    ("x-ratelimit-limit-requests", "x_ratelimit_limit_requests"),
    ("x-ratelimit-limit-tokens", "x_ratelimit_limit_tokens"),
    (
        "x-ratelimit-remaining-requests",
        "x_ratelimit_remaining_requests",
    ),
    (
        "x-ratelimit-remaining-tokens",
        "x_ratelimit_remaining_tokens",
    ),
    ("x-ratelimit-reset-requests", "x_ratelimit_reset_requests"),
    ("x-ratelimit-reset-tokens", "x_ratelimit_reset_tokens"),
];

#[derive(Debug, Error)]
pub enum HiddenParamsError {
    #[error("hidden_params.additional_headers must be an object or null")]
    AdditionalHeadersType,
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

#[must_use]
pub fn get_additional_headers(
    additional: Option<&IndexMap<String, Json>>,
) -> Option<AdditionalHeaders> {
    let present = additional?;
    let mut converted = AdditionalHeaders::new();
    for (wire, field) in TYPED_HEADERS {
        if let Some(value) = present.get(wire) {
            converted.insert(
                field.to_owned(),
                int_of(value).unwrap_or_else(|| value.clone()),
            );
        }
    }
    for (key, value) in present {
        if !TYPED_HEADERS
            .iter()
            .any(|(wire, _field)| key.to_lowercase() == *wire)
        {
            converted.insert(key.clone(), value.clone());
        }
    }
    Some(converted)
}

pub fn get_hidden_params(
    hidden: Option<&Json>,
) -> Result<StandardLoggingHiddenParams, HiddenParamsError> {
    let Some(Json::Object(fields)) = hidden else {
        return Ok(StandardLoggingHiddenParams::default());
    };
    let mut copied = IndexMap::new();
    for key in HIDDEN_PARAM_KEYS {
        let Some(entry) = fields.get(key) else {
            continue;
        };
        let converted = if key == "additional_headers" {
            encoded_additional_headers(entry)?
        } else {
            entry.clone()
        };
        copied.insert(key.to_owned(), converted);
    }
    Ok(decode("hidden_params", copied)?)
}

fn encoded_additional_headers(value: &Json) -> Result<Json, HiddenParamsError> {
    match value {
        Json::Null => Ok(Json::Null),
        Json::Object(headers) => {
            Ok(get_additional_headers(Some(headers)).map_or(Json::Null, Json::Object))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(HiddenParamsError::AdditionalHeadersType)
        }
    }
}

// Python's int(): a bool counts, a number truncates, and a numeric string parses
fn int_of(value: &Json) -> Option<Json> {
    match value {
        Json::Bool(flag) => Some(Json::Number(Number::from(u8::from(*flag)))),
        Json::Number(number) => int_of_number(number),
        Json::Text(text) => int_of_text(text),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn int_of_number(number: &Number) -> Option<Json> {
    if let Some(integer) = number.as_i64() {
        return Some(Json::Number(Number::from(integer)));
    }
    if let Some(integer) = number.as_u64() {
        return Some(Json::Number(Number::from(integer)));
    }
    let literal = number.as_str();
    if !literal.is_empty()
        && literal
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return Some(Json::Number(number.clone()));
    }
    let value = number.as_f64()?;
    if !value.is_finite() {
        return None;
    }
    serde_json::from_str::<Number>(&format!("{:.0}", value.trunc()))
        .ok()
        .map(Json::Number)
}

fn int_of_text(text: &str) -> Option<Json> {
    let cleaned = integer_text(text)?;
    let number = serde_json::from_str::<Number>(&cleaned).ok()?;
    Some(Json::Number(number))
}

fn integer_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let digits = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    if digits.is_empty() {
        return None;
    }
    let bytes = digits.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'0'..=b'9' => {}
            b'_' => {
                let before = index
                    .checked_sub(1)
                    .and_then(|position| bytes.get(position));
                let after = index
                    .checked_add(1)
                    .and_then(|position| bytes.get(position));
                let surrounded =
                    before.is_some_and(u8::is_ascii_digit) && after.is_some_and(u8::is_ascii_digit);
                if !surrounded {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let mut cleaned = String::with_capacity(digits.len());
    if trimmed.starts_with('-') {
        cleaned.push('-');
    }
    for byte in bytes {
        if *byte != b'_' {
            cleaned.push(char::from(*byte));
        }
    }
    Some(cleaned)
}

mod tests;
