use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use num_traits::cast::cast;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::params::error::ParamsError;

#[must_use]
pub const fn as_str(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub const fn as_object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// LiteLLM's `a or b` picks the second value unless the first is null, false, zero or empty
#[must_use]
pub fn is_set(value: &Json) -> bool {
    !match value {
        Json::Null | Json::Bool(false) => true,
        Json::Bool(true) => false,
        Json::Number(number) => number.as_f64() == Some(0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}

// LiteLLM passes an int through untouched and rounds anything else, so the literal decides which
#[must_use]
pub fn is_integer_literal(number: &serde_json::Number) -> bool {
    !number.to_string().contains(['.', 'e', 'E'])
}

// Python's int(value) over the shapes a request can carry: booleans and integer literals
pub fn integer(value: &Json) -> Option<i64> {
    match value {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) if is_integer_literal(number) => number.as_i64(),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// CPython's round() for floats rounds half to even, which f64::round does not
pub fn rounded_tokens(number: &serde_json::Number) -> Option<i64> {
    let parsed = number.as_f64()?;
    cast::<f64, i64>(parsed.round_ties_even()).map(|tokens| tokens.max(1))
}

#[must_use]
pub fn prompt_json(value: &Json) -> PromptJson {
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

pub fn encode<T>(value: &T) -> Result<Json, ParamsError>
where
    T: Serialize,
{
    let serialized = serde_json::to_value(value).map_err(ParamsError::Encode)?;
    serde_json::from_value(serialized).map_err(ParamsError::Encode)
}

pub fn decode<T>(value: &Json) -> Result<T, ParamsError>
where
    T: DeserializeOwned,
{
    let serialized = serde_json::to_value(value).map_err(ParamsError::Encode)?;
    serde_json::from_value(serialized).map_err(ParamsError::Decode)
}

// B2202's base helpers take serde_json's map; key order is outside the compatibility contract
pub fn base_params(params: &IndexMap<String, Json>) -> Result<Map<String, Value>, ParamsError> {
    let encoded = serde_json::to_value(params).map_err(ParamsError::Encode)?;
    match encoded {
        Value::Object(fields) => Ok(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(ParamsError::NotAnObject)
        }
    }
}

pub fn json_params(fields: Map<String, Value>) -> Result<IndexMap<String, Json>, ParamsError> {
    serde_json::from_value(Value::Object(fields)).map_err(ParamsError::Decode)
}
