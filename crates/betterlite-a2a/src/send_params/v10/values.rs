use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use betterlite_core::agents::json::AgentJson;
use num_traits::ToPrimitive as _;

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::JsonMap;

// protobuf stores every Struct and Value number as a double, so an integer comes back widened
pub(super) fn normalize_map(
    entries: JsonMap,
    field: &'static str,
) -> Result<JsonMap, SendParamsError> {
    let mut normalized = JsonMap::new();
    for (key, value) in entries {
        normalized.insert(key, normalize_value(&value, field)?);
    }
    Ok(normalized)
}

pub(super) fn normalize_value(
    value: &AgentJson,
    field: &'static str,
) -> Result<AgentJson, SendParamsError> {
    match value {
        AgentJson::Number(number) => number
            .as_f64()
            .filter(|float| float.is_finite())
            .map_or_else(
                || {
                    Err(SendParamsError::InvalidValue {
                        field,
                        expected: "double",
                    })
                },
                |float| Ok(AgentJson::from(float)),
            ),
        AgentJson::Array(items) => items
            .iter()
            .map(|item| normalize_value(item, field))
            .collect::<Result<Vec<_>, _>>()
            .map(AgentJson::Array),
        AgentJson::Object(entries) => {
            let mut normalized = entries.clone();
            for (key, entry) in entries {
                normalized.insert(key.clone(), normalize_value(entry, field)?);
            }
            Ok(AgentJson::Object(normalized))
        }
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::String(_) => Ok(value.clone()),
    }
}

pub(super) fn integer_from_number(
    number: &serde_json::Number,
    field: &'static str,
) -> Result<i64, SendParamsError> {
    if let Some(value) = number.as_i64() {
        return Ok(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).map_err(|_error| SendParamsError::InvalidValue {
            field,
            expected: "64-bit integer",
        });
    }
    let Some(float) = number.as_f64() else {
        return Err(SendParamsError::InvalidValue {
            field,
            expected: "64-bit integer",
        });
    };
    if float.fract().to_bits() != 0.0_f64.to_bits() {
        return Err(SendParamsError::InvalidValue {
            field,
            expected: "integer",
        });
    }
    float.to_i64().ok_or(SendParamsError::InvalidValue {
        field,
        expected: "64-bit integer",
    })
}

// protobuf reads bytes in the standard or url-safe alphabet, padded or not, and skips other characters
pub(super) fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut cleaned = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => cleaned.push(character),
            '-' => cleaned.push('+'),
            '_' => cleaned.push('/'),
            _ => {}
        }
    }
    STANDARD_NO_PAD.decode(cleaned.as_bytes()).ok()
}
