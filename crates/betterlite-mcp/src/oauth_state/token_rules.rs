mod tests;

use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use serde_json::Number;
use thiserror::Error;

const SCIENTIFIC_LOW: f64 = 1e-4;
const SCIENTIFIC_HIGH: f64 = 1e16;

#[derive(Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct TokenValidationFailure {
    pub server_id: String,
    pub field: String,
    pub message: String,
}

pub fn validate_token_response(
    token_response: &JsonValue,
    rules: &BTreeMap<String, JsonValue>,
    server_id: &str,
) -> Result<(), TokenValidationFailure> {
    for (key, expected) in rules {
        let Some(actual) = token_field(token_response, key) else {
            let message = format!("OAuth token rejected: required field '{key}' is absent");
            return Err(TokenValidationFailure {
                server_id: server_id.to_owned(),
                field: key.clone(),
                message,
            });
        };
        if normalize_for_comparison(actual) != normalize_for_comparison(expected) {
            let message = format!(
                "OAuth token rejected: '{key}' = '{}', expected '{}'",
                python_text(actual),
                python_text(expected)
            );
            return Err(TokenValidationFailure {
                server_id: server_id.to_owned(),
                field: key.clone(),
                message,
            });
        }
    }
    Ok(())
}

// A top-level key is tried whole first; the dot walk starts only when that value is absent or null.
fn token_field<'value>(token_response: &'value JsonValue, key: &str) -> Option<&'value JsonValue> {
    match token_response {
        JsonValue::Object(fields) => match fields.get(key) {
            Some(JsonValue::Null) | None => walk_dot_path(token_response, key),
            Some(value) => Some(value),
        },
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn walk_dot_path<'value>(
    token_response: &'value JsonValue,
    key: &str,
) -> Option<&'value JsonValue> {
    if !key.contains('.') {
        return None;
    }
    let mut current = token_response;
    for part in key.split('.') {
        let JsonValue::Object(fields) = current else {
            return None;
        };
        current = fields.get(part)?;
    }
    match current {
        JsonValue::Null => None,
        JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Some(current),
    }
}

fn normalize_for_comparison(value: &JsonValue) -> String {
    match value {
        JsonValue::Bool(flag) => flag.to_string(),
        JsonValue::Null
        | JsonValue::String(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => python_text(value),
    }
}

// Python's str() of a JSON value: booleans and null in their Python spelling, numbers via the float repr.
fn python_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_owned(),
        JsonValue::Bool(true) => "True".to_owned(),
        JsonValue::Bool(false) => "False".to_owned(),
        JsonValue::Number(number) => number_text(number),
        JsonValue::String(text) => text.clone(),
        JsonValue::Array(_) | JsonValue::Object(_) => match serde_json::to_string(value) {
            Ok(text) => text,
            Err(_error) => String::new(),
        },
    }
}

fn number_text(number: &Number) -> String {
    let literal = number.to_string();
    if !literal.contains(['.', 'e', 'E']) {
        return match literal.parse::<i64>() {
            Ok(value) => value.to_string(),
            Err(_error) => literal,
        };
    }
    let Ok(value) = literal.parse::<f64>() else {
        return literal;
    };
    python_float_text(value)
}

fn python_float_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    let magnitude = value.abs();
    if magnitude != 0.0 && !(SCIENTIFIC_LOW..SCIENTIFIC_HIGH).contains(&magnitude) {
        return python_exponent_text(value);
    }
    let text = value.to_string();
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

fn python_exponent_text(value: f64) -> String {
    let scientific = format!("{value:e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let sign = if exponent.starts_with('-') { '-' } else { '+' };
    let digits = exponent.trim_start_matches(['-', '+']);
    let padded = if digits.len() < 2 {
        format!("0{digits}")
    } else {
        digits.to_owned()
    };
    format!("{mantissa}e{sign}{padded}")
}
