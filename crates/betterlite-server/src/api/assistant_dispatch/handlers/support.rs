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

pub(super) fn optional<T>(
    kwargs: &BTreeMap<String, Json>,
    name: &str,
) -> Result<Option<T>, AssistantDispatchError>
where
    T: DeserializeOwned,
{
    let Some(value) = kwargs.get(name) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    parse_value(value).map(Some)
}

pub(super) fn subset(kwargs: &BTreeMap<String, Json>, names: &[&str]) -> BTreeMap<String, Json> {
    let mut kept = BTreeMap::new();
    for name in names {
        if let Some(value) = kwargs.get(*name) {
            kept.insert((*name).to_owned(), value.clone());
        }
    }
    kept
}

pub(super) fn parse_value<T>(value: &Json) -> Result<T, AssistantDispatchError>
where
    T: DeserializeOwned,
{
    let bytes = serde_json::to_vec(value).map_err(|error| AssistantDispatchError::Invalid {
        message: error.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|error| AssistantDispatchError::Invalid {
        message: error.to_string(),
    })
}
