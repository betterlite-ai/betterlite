use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use thiserror::Error;

const METADATA: &str = "metadata";
const LITELLM_METADATA: &str = "litellm_metadata";
const MODEL: &str = "model";
const MODEL_ID: &str = "id";
const MODEL_INFO: &str = "model_info";
const MODEL_INFO_FIELD: &str = "metadata.model_info";
const TEAM_ID: &str = "user_api_key_team_id";
const GUARDRAILS: &str = "guardrails";

#[derive(Debug, Clone)]
pub struct Deployment {
    pub guardrails: Option<JsonValue>,
}

pub trait DeploymentGuardrails {
    fn get_deployment(&self, model_id: &str) -> Option<Deployment>;
    fn get_model_list(&self, model_name: &str, team_id: Option<&str>) -> Vec<Deployment>;
}

#[derive(Debug, Error)]
pub enum ModelGuardrailsError {
    #[error("'{field}' must be an object")]
    NotAnObject { field: &'static str },
}

pub fn merge_model_level_guardrails(
    data: &JsonMap,
    deployments: Option<&dyn DeploymentGuardrails>,
    trust_client_model_info: bool,
    model_alias: Option<&str>,
) -> Result<JsonMap, ModelGuardrailsError> {
    let Some(finder) = deployments else {
        return Ok(data.clone());
    };
    let metadata = metadata_object(data, METADATA)?;
    let litellm_metadata = metadata_object(data, LITELLM_METADATA)?;
    let model_info = model_info_object(metadata)?;
    let trusted_id = model_info
        .and_then(|info| info.get(MODEL_ID))
        .filter(|_| trust_client_model_info)
        .filter(|id_value| !id_value.is_null());
    let model_level = match trusted_id {
        Some(id_value) => {
            let deployment = id_value
                .as_str()
                .and_then(|model_id| finder.get_deployment(model_id));
            let Some(deployment) = deployment else {
                return Ok(data.clone());
            };
            trusted_guardrails(deployment.guardrails.as_ref())
        }
        None => alias_guardrails(
            data,
            finder,
            model_alias,
            resolve_team_id(metadata, litellm_metadata),
        ),
    };
    let Some(model_level) = model_level else {
        return Ok(data.clone());
    };
    merge_guardrails_with_existing(data, &JsonValue::Array(model_level))
}

pub fn merge_guardrails_with_existing(
    data: &JsonMap,
    model_level_guardrails: &JsonValue,
) -> Result<JsonMap, ModelGuardrailsError> {
    let mut modified = data.clone();
    let mut metadata = match modified.remove(METADATA) {
        None | Some(JsonValue::Null) => JsonMap::new(),
        Some(JsonValue::Object(metadata)) => metadata,
        Some(_) => {
            return Err(ModelGuardrailsError::NotAnObject { field: METADATA });
        }
    };
    let existing = trusted_guardrails(metadata.get(GUARDRAILS)).unwrap_or_default();
    let model_level = trusted_guardrails(Some(model_level_guardrails)).unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut merged = Vec::new();
    for entry in existing.into_iter().chain(model_level) {
        if seen.insert(entry.to_string()) {
            merged.push(entry);
        }
    }
    metadata.insert(GUARDRAILS.to_owned(), JsonValue::Array(merged));
    modified.insert(METADATA.to_owned(), JsonValue::Object(metadata));
    Ok(modified)
}

fn metadata_object(
    data: &JsonMap,
    field: &'static str,
) -> Result<Option<&JsonMap>, ModelGuardrailsError> {
    match data.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(bucket)) => Ok(Some(bucket)),
        Some(_) => Err(ModelGuardrailsError::NotAnObject { field }),
    }
}

fn model_info_object(metadata: Option<&JsonMap>) -> Result<Option<&JsonMap>, ModelGuardrailsError> {
    let Some(bucket) = metadata else {
        return Ok(None);
    };
    match bucket.get(MODEL_INFO) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(info)) => Ok(Some(info)),
        Some(_) => Err(ModelGuardrailsError::NotAnObject {
            field: MODEL_INFO_FIELD,
        }),
    }
}

fn resolve_team_id<'data>(
    metadata: Option<&'data JsonMap>,
    litellm_metadata: Option<&'data JsonMap>,
) -> Option<&'data str> {
    let candidates = [
        metadata.and_then(|bucket| bucket.get(TEAM_ID)),
        litellm_metadata.and_then(|bucket| bucket.get(TEAM_ID)),
    ];
    candidates
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .find(|team| !team.is_empty())
}

fn alias_guardrails(
    data: &JsonMap,
    deployments: &dyn DeploymentGuardrails,
    model_alias: Option<&str>,
    team_id: Option<&str>,
) -> Option<Vec<JsonValue>> {
    let alias = model_alias.or_else(|| data.get(MODEL).and_then(JsonValue::as_str))?;
    if alias.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut union = Vec::new();
    for deployment in deployments.get_model_list(alias, team_id) {
        let Some(entries) = alias_guardrails_entry(deployment.guardrails.as_ref()) else {
            continue;
        };
        for entry in entries {
            if seen.insert(entry.to_string()) {
                union.push(entry);
            }
        }
    }
    if union.is_empty() { None } else { Some(union) }
}

fn alias_guardrails_entry(guardrails: Option<&JsonValue>) -> Option<Vec<JsonValue>> {
    match guardrails {
        Some(JsonValue::String(name)) => Some(vec![JsonValue::String(name.clone())]),
        Some(JsonValue::Array(items)) => Some(items.clone()),
        None
        | Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Number(_))
        | Some(JsonValue::Object(_)) => None,
    }
}

fn trusted_guardrails(guardrails: Option<&JsonValue>) -> Option<Vec<JsonValue>> {
    match guardrails {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Array(items)) => Some(items.clone()),
        Some(value) => Some(vec![value.clone()]),
    }
}
