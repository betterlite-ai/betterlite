use std::time::Duration;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::ConvertError;

pub(super) type Params = IndexMap<String, Json>;
pub(super) type TextMap = IndexMap<String, String>;
pub(super) type Metadata = IndexMap<String, JsonValue>;

pub(super) const TIMEOUT: &str = "a number of seconds";
pub(super) const STRING: &str = "a string";
pub(super) const INTEGER: &str = "an integer";
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

// the OpenAI API types seconds as a string, and an integer is sent as given rather than formatted
pub(super) fn seconds(arguments: &RequestMap, name: &str) -> Result<Option<String>, ConvertError> {
    if let Some(Json::Number(value)) = arguments.get(name) {
        return Ok(Some(value.to_string()));
    }
    text(arguments, name)
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

pub(super) fn duration(value: Option<f64>) -> Result<Option<Duration>, ConvertError> {
    value
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|source| ConvertError::Timeout { source })
        })
        .transpose()
}

pub(super) fn params(arguments: &RequestMap, name: &str) -> Result<Option<Params>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(entries)) => Ok(Some(entries.clone())),
        Some(_) => Err(failed(name, MAP)),
    }
}

pub(super) fn texts(arguments: &RequestMap, name: &str) -> Result<Option<TextMap>, ConvertError> {
    let Some(entries) = params(arguments, name)? else {
        return Ok(None);
    };
    let pairs = entries.into_iter().map(|(key, value)| match value {
        Json::Text(text) => Ok((key, text)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(failed(name, TEXT_MAP))
        }
    });
    pairs.collect::<Result<TextMap, _>>().map(Some)
}

pub(super) fn meta(arguments: &RequestMap, name: &str) -> Result<Option<Metadata>, ConvertError> {
    let Some(value) = params(arguments, name)? else {
        return Ok(None);
    };
    json(&value).map(Some)
}

// B525's wire types and this crate's own records are the same JSON, so each conversion is one round trip
pub(crate) fn json<T, V>(value: &V) -> Result<T, ConvertError>
where
    T: DeserializeOwned,
    V: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(ConvertError::from)?;
    let decoded = serde_json::from_value(encoded).map_err(ConvertError::from)?;
    Ok(decoded)
}
