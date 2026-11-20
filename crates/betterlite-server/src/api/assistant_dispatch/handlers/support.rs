use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;

use super::super::error::AssistantDispatchError;

// the wrapper's named arguments: the installed function raises TypeError before doing any work
pub(super) fn require(
    kwargs: &BTreeMap<String, Json>,
    function: &str,
    names: &[&str],
) -> Result<(), AssistantDispatchError> {
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !kwargs.contains_key(*name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AssistantDispatchError::missing_arguments(
            function, &missing,
        ))
    }
}

pub(super) fn text(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<String>, AssistantDispatchError> {
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AssistantDispatchError::invalid(name))
        }
    }
}

pub(super) fn required_text(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
    function: &str,
) -> Result<String, AssistantDispatchError> {
    text(kwargs, name)?.ok_or_else(|| AssistantDispatchError::missing_arguments(function, &[name]))
}

pub(super) fn optional<T: DeserializeOwned>(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<T>, AssistantDispatchError> {
    match kwargs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => parse_value(value).map(Some),
    }
}

pub(super) fn subset(kwargs: &BTreeMap<String, Json>, names: &[&str]) -> BTreeMap<String, Json> {
    names
        .iter()
        .filter_map(|name| {
            kwargs
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect()
}

pub(super) fn parse<T: DeserializeOwned>(
    fields: &BTreeMap<String, Json>,
) -> Result<T, AssistantDispatchError> {
    let bytes = serde_json::to_vec(fields)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))
}

pub(super) fn parse_value<T: DeserializeOwned>(value: &Json) -> Result<T, AssistantDispatchError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AssistantDispatchError::invalid_message(error.to_string()))
}
