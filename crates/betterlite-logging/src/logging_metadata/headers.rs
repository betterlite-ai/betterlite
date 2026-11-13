use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::fields::payload::records::{AdditionalHeaders, StandardLoggingHiddenParams};
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::keys::HIDDEN_PARAM_KEYS;

// StandardLoggingAdditionalHeaders fields, each read from the header spelled with dashes
const TYPED_HEADERS: [&str; 6] = [
    "x-ratelimit-limit-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-remaining-tokens",
    "x-ratelimit-reset-requests",
    "x-ratelimit-reset-tokens",
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
    for wire in TYPED_HEADERS {
        if let Some(value) = present.get(wire) {
            let converted_value = int_of(value).unwrap_or_else(|| value.clone());
            converted.insert(wire.replace('-', "_"), converted_value);
        }
    }
    for (key, value) in present {
        if !TYPED_HEADERS.contains(&key.to_lowercase().as_str()) {
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

// LiteLLM's int(): a number truncates, and numeric text parses
fn int_of(value: &Json) -> Option<Json> {
    let number = match value {
        Json::Number(number) if number.is_f64() => {
            serde_json::from_str::<Number>(&format!("{:.0}", number.as_f64()?.trunc())).ok()?
        }
        Json::Number(number) => number.clone(),
        Json::Text(text) => Number::from(text.trim().parse::<i64>().ok()?),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => return None,
    };
    Some(Json::Number(number))
}

mod tests;
