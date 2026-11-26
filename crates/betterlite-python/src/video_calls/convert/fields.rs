use std::time::Duration;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::{ConvertError, SettingsWire};

const TIMEOUT: &str = "a number of seconds";
const STRING: &str = "a string";
const INTEGER: &str = "an integer";
const MAP: &str = "a map";
const TEXT_MAP: &str = "a map of strings";

pub(super) fn failed(name: &str, expected: &'static str) -> ConvertError {
    ConvertError::Field {
        name: name.to_owned(),
        expected,
    }
}

pub(super) fn text(arguments: &RequestMap, name: &str) -> Result<Option<String>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(_) => Err(failed(name, STRING)),
    }
}

pub(super) fn required_text(arguments: &RequestMap, name: &str) -> Result<String, ConvertError> {
    match text(arguments, name)? {
        Some(value) => Ok(value),
        None => Err(failed(name, STRING)),
    }
}

// the OpenAI API types seconds as a string, and an integer is sent as given rather than formatted
pub(super) fn seconds_value(
    arguments: &RequestMap,
    name: &str,
) -> Result<Option<String>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Number(value)) => Ok(Some(value.to_string())),
        Some(_) => Err(failed(name, STRING)),
    }
}

pub(super) fn integer(arguments: &RequestMap, name: &str) -> Result<Option<i64>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| failed(name, INTEGER)),
        Some(_) => Err(failed(name, INTEGER)),
    }
}

pub(super) fn number(arguments: &RequestMap, name: &str) -> Result<Option<f64>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| failed(name, TIMEOUT)),
        Some(_) => Err(failed(name, TIMEOUT)),
    }
}

pub(super) fn text_map(
    arguments: &RequestMap,
    name: &str,
) -> Result<Option<IndexMap<String, String>>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(entries)) => entries
            .iter()
            .map(|(key, value)| match value {
                Json::Text(text) => Ok((key.clone(), text.clone())),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(failed(name, TEXT_MAP))
                }
            })
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(failed(name, TEXT_MAP)),
    }
}

pub(super) fn params(
    arguments: &RequestMap,
    name: &str,
) -> Result<Option<IndexMap<String, Json>>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(entries)) => Ok(Some(entries.clone())),
        Some(_) => Err(failed(name, MAP)),
    }
}

pub(super) fn metadata(
    arguments: &RequestMap,
    name: &str,
) -> Result<Option<IndexMap<String, JsonValue>>, ConvertError> {
    let Some(value) = params(arguments, name)? else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(value).map_err(|source| ConvertError::Json { source })?;
    serde_json::from_value(encoded)
        .map(Some)
        .map_err(|source| ConvertError::Json { source })
}

pub(super) fn duration(value: Option<f64>) -> Result<Option<Duration>, ConvertError> {
    value
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|source| ConvertError::Timeout { source })
        })
        .transpose()
}

// timeout or REQUEST_TIMEOUT: an explicit null or zero takes the request timeout further down
pub(crate) fn drop_params(
    arguments: &RequestMap,
    settings: &SettingsWire,
) -> Result<bool, ConvertError> {
    let given = match arguments.get("drop_params") {
        None | Some(Json::Null) => None,
        Some(Json::Bool(value)) => Some(*value),
        Some(_) => return Err(failed("drop_params", "a boolean")),
    };
    Ok(given.unwrap_or(false) || settings.drop_params.unwrap_or(false))
}

pub(super) fn secret(
    arguments: &RequestMap,
    name: &str,
) -> Result<Option<SecretString>, ConvertError> {
    match text(arguments, name)? {
        Some(value) => Ok(Some(SecretString::from(value))),
        None => Ok(None),
    }
}
