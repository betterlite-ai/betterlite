use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;

use super::ConvertError;

pub(crate) type Params = IndexMap<String, Json>;
pub(crate) type TextMap = IndexMap<String, String>;

pub(crate) const TIMEOUT: &str = "a number of seconds";
pub(crate) const STRING: &str = "a string";
const MAP: &str = "a map";
const TEXT_MAP: &str = "a map of strings";

pub(crate) fn failed(name: &str, expected: &'static str) -> ConvertError {
    ConvertError::Field {
        name: name.to_owned(),
        expected,
    }
}

pub(crate) fn text(arguments: &RequestMap, name: &str) -> Result<Option<String>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(_) => Err(failed(name, STRING)),
    }
}

pub(crate) fn number(arguments: &RequestMap, name: &str) -> Result<Option<f64>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| failed(name, TIMEOUT)),
        Some(_) => Err(failed(name, TIMEOUT)),
    }
}

pub(crate) fn duration(value: Option<f64>) -> Result<Option<Duration>, ConvertError> {
    value
        .map(|seconds| {
            Duration::try_from_secs_f64(seconds).map_err(|source| ConvertError::Timeout { source })
        })
        .transpose()
}

pub(crate) fn params(arguments: &RequestMap, name: &str) -> Result<Option<Params>, ConvertError> {
    match arguments.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(entries)) => Ok(Some(entries.clone())),
        Some(_) => Err(failed(name, MAP)),
    }
}

pub(crate) fn texts(arguments: &RequestMap, name: &str) -> Result<Option<TextMap>, ConvertError> {
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
