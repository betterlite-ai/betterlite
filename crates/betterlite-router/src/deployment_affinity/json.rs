use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::error::DeploymentAffinityError;

pub const PREVIOUS_RESPONSE_ID: &str = "previous_response_id";
const DEPLOYMENT_MODEL_NAME: &str = "deployment_model_name";
const MODEL_INFO: &str = "model_info";

// _iter_metadata_dicts: both channels, in LiteLLM's order, never one instead of the other
fn metadata_dicts(kwargs: &JsonObject) -> Vec<&IndexMap<String, Json>> {
    let mut dicts = Vec::with_capacity(2);
    for channel in ["litellm_metadata", "metadata"] {
        if let Some(Json::Object(metadata)) = kwargs.get(channel) {
            dicts.push(metadata);
        }
    }
    dicts
}

// the first non-empty deployment_model_name across the metadata channels
#[must_use]
pub fn deployment_model_name(kwargs: &JsonObject) -> Option<&str> {
    metadata_dicts(kwargs).into_iter().find_map(|metadata| {
        match metadata.get(DEPLOYMENT_MODEL_NAME) {
            Some(Json::Text(name)) => (!name.is_empty()).then_some(name.as_str()),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        }
    })
}

// the model_info fallback: the selected deployment's own dict first, then the first metadata channel holding one
#[must_use]
pub fn metadata_model_info(kwargs: &JsonObject) -> Option<&JsonObject> {
    metadata_dicts(kwargs)
        .into_iter()
        .find_map(|metadata| metadata.get(MODEL_INFO).and_then(json_object))
}

fn json_object(slot: &Json) -> Option<&JsonObject> {
    match slot {
        Json::Object(value) => Some(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// prior response ids are declared strings; another type names the field instead of being read as text
pub fn previous_response_id(kwargs: &JsonObject) -> Result<Option<&str>, DeploymentAffinityError> {
    match kwargs.get(PREVIOUS_RESPONSE_ID) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(previous)) => Ok(Some(previous.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DeploymentAffinityError::RequestNotText {
                field: PREVIOUS_RESPONSE_ID,
            })
        }
    }
}
