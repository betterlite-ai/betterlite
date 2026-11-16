use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::databricks_chat::error::DatabricksChatError;

// The Anthropic capability seams and the base OpenAI helpers carry decisions::Json maps; the
// provider surface carries prompts::Object, so both sides convert through serde_json here.
pub(crate) type Params = IndexMap<String, DecisionJson>;

pub(crate) fn to_params(object: &Object) -> Result<Params, DatabricksChatError> {
    let encoded =
        serde_json::to_value(object).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

pub(crate) fn from_params(params: &Params) -> Result<Object, DatabricksChatError> {
    let encoded =
        serde_json::to_value(params).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

// The caller's type names the wire map it hands to a base helper, so it stays unspelled here.
pub(crate) fn to_wire<T>(params: &Params) -> Result<T, DatabricksChatError>
where
    T: DeserializeOwned,
{
    let encoded =
        serde_json::to_value(params).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

pub(crate) fn decision_json(value: &Json) -> Result<DecisionJson, DatabricksChatError> {
    let encoded =
        serde_json::to_value(value).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

pub(crate) fn prompt_json(value: &DecisionJson) -> Result<Json, DatabricksChatError> {
    let encoded =
        serde_json::to_value(value).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

pub(crate) fn encode_value<T>(value: &T) -> Result<Json, DatabricksChatError>
where
    T: Serialize + ?Sized,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| DatabricksChatError::Encode(source))?;
    serde_json::from_value(encoded).map_err(DatabricksChatError::Decode)
}

pub(crate) fn text(value: &DecisionJson) -> Option<&str> {
    match value {
        DecisionJson::Text(text) => Some(text.as_str()),
        DecisionJson::Null
        | DecisionJson::Bool(_)
        | DecisionJson::Number(_)
        | DecisionJson::List(_)
        | DecisionJson::Object(_) => None,
    }
}

pub(crate) fn object(value: &DecisionJson) -> Option<&Params> {
    match value {
        DecisionJson::Object(fields) => Some(fields),
        DecisionJson::Null
        | DecisionJson::Bool(_)
        | DecisionJson::Number(_)
        | DecisionJson::Text(_)
        | DecisionJson::List(_) => None,
    }
}

// A short rendering for the chunk parser's KeyError text; Python's repr is not ported.
pub(crate) fn render(value: &Json) -> String {
    match serde_json::to_string(value) {
        Ok(text) => text,
        Err(_) => String::new(),
    }
}
