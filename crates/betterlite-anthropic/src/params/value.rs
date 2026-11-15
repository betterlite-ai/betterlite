use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;

use crate::params::error::ParamsError;

#[must_use]
pub(crate) const fn as_str(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub(crate) const fn as_object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// Python's int(value) over the shapes a request can carry: booleans and int-shaped numbers
pub(crate) fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub(crate) fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), prompt_json(entry)))
                .collect(),
        ),
    }
}

// B2202's base helpers take serde_json's map; the caller's type names the wire map it hands over
pub(crate) fn to_wire<T>(params: &IndexMap<String, Json>) -> Result<T, ParamsError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_value(params).map_err(ParamsError::Encode)?;
    if !encoded.is_object() {
        return Err(ParamsError::NotAnObject);
    }
    serde_json::from_value(encoded).map_err(ParamsError::Decode)
}

// A mapped record as the Json tree the parameter maps carry
pub(crate) fn encode<T>(record: &T) -> Result<Json, ParamsError>
where
    T: serde::Serialize + ?Sized,
{
    serde_json::to_value(record)
        .and_then(serde_json::from_value)
        .map_err(ParamsError::Encode)
}
