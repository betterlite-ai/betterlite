use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;

// CredentialLiteLLMParams, in its declared order, so a dump keeps the same fields LiteLLM's model keeps.
const CREDENTIAL_FIELDS: &str = "api_key api_base api_version azure_ad_token tenant_id client_id client_secret azure_scope azure_username azure_password vertex_project vertex_location vertex_credentials region_name gcs_bucket_name bucket_name aws_access_key_id aws_secret_access_key aws_session_token aws_region_name aws_session_name aws_profile_name aws_role_name aws_web_identity_token aws_sts_endpoint aws_external_id aws_session_tags aws_bedrock_runtime_endpoint aws_bedrock_project_id s3_bucket_name s3_endpoint_url s3_region_name s3_access_key_id s3_secret_access_key s3_encryption_key_id s3_bucket_owner aws_batch_role_arn s3_output_bucket_name bedrock_tags watsonx_region_name";

// Fields pydantic validates as str | dict, or as a list, keep more than plain text.
const TEXT_OR_OBJECT: [&str; 1] = ["vertex_credentials"];
const TEXT_OR_LIST: [&str; 2] = ["aws_session_tags", "bedrock_tags"];

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the deployment credential field {field} is not {expected}")]
    FieldType {
        field: String,
        expected: &'static str,
    },
    #[error("the deployment has no model to hand its credentials to")]
    MissingModel,
    #[error("the deployment credentials could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// CredentialLiteLLMParams.model_validate(...).model_dump(exclude_none=True): declared fields only, nulls gone.
pub fn credential_params(litellm_params: &JsonObject) -> Result<JsonObject, CredentialError> {
    let mut credentials = JsonObject::new();
    for field in CREDENTIAL_FIELDS.split(' ') {
        let Some(value) = litellm_params
            .get(field)
            .filter(|value| !matches!(value, Json::Null))
        else {
            continue;
        };
        let accepted = match value {
            Json::Text(_) => true,
            Json::Object(_) => TEXT_OR_OBJECT.contains(&field),
            Json::List(_) => TEXT_OR_LIST.contains(&field),
            Json::Null | Json::Bool(_) | Json::Number(_) => false,
        };
        if !accepted {
            return Err(CredentialError::FieldType {
                field: field.to_owned(),
                expected: "a string, an object or a list",
            });
        }
        credentials.insert(field.to_owned(), value.clone());
    }
    Ok(credentials)
}

// _deployment_usable_by_team: a team deployment serves only its own team, an unowned one is shared
fn usable_by_team(deployment: &DeploymentSpec, team_id: Option<&str>) -> bool {
    match (deployment.model_info.get("team_id"), team_id) {
        (Some(Json::Text(owner)), Some(caller)) => owner == caller,
        (Some(Json::Text(_)), None) => false,
        _ => true,
    }
}

fn first_usable<'catalog>(
    catalog: &'catalog DeploymentCatalog,
    indices: Option<&Vec<usize>>,
    team_id: Option<&str>,
) -> Option<&'catalog DeploymentSpec> {
    indices?
        .iter()
        .filter_map(|index| catalog.models.get(*index))
        .find(|deployment| usable_by_team(deployment, team_id))
}

// by id, then model name, then the team's public name, then a wildcard, and nothing when the match is paused
#[must_use]
pub fn credential_deployment<'catalog>(
    catalog: &'catalog DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Option<&'catalog DeploymentSpec> {
    let deployment = catalog
        .id_index
        .get(model_id)
        .and_then(|index| catalog.models.get(*index))
        .or_else(|| first_usable(catalog, catalog.name_index.get(model_id), team_id))
        .or_else(|| {
            let team = team_id?;
            let indices = catalog
                .team_index
                .get(&(team.to_owned(), model_id.to_owned()))?;
            let index = indices.first()?;
            catalog.models.get(*index)
        })
        .or_else(|| {
            let matched = wildcard_model_id(catalog, model_id, team_id)?;
            let index = catalog.id_index.get(&matched)?;
            catalog.models.get(*index)
        })?;
    if matches!(deployment.model_info.get("blocked"), Some(Json::Bool(true))) {
        return None;
    }
    Some(deployment)
}

// the team's own patterns answer first; a global pattern only counts when the deployment it names is usable
fn wildcard_model_id(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
) -> Option<String> {
    if let Some(patterns) = team_id.and_then(|team| catalog.team_pattern_routers.get(team)) {
        let first = patterns.router.route(model_id, None)?.into_iter().next()?;
        return first.model_id;
    }
    let matched = catalog.pattern_router.route(model_id, None)?;
    let usable = matched.into_iter().find(|candidate| {
        candidate
            .model_id
            .as_deref()
            .and_then(|id| catalog.id_index.get(id))
            .and_then(|index| catalog.models.get(*index))
            .is_some_and(|deployment| usable_by_team(deployment, team_id))
    })?;
    usable.model_id
}

pub fn deployment_credentials(
    catalog: &DeploymentCatalog,
    model_id: &str,
) -> Result<Option<JsonObject>, CredentialError> {
    let Some(deployment) = catalog
        .id_index
        .get(model_id)
        .and_then(|index| catalog.models.get(*index))
    else {
        return Ok(None);
    };
    if matches!(deployment.model_info.get("blocked"), Some(Json::Bool(true))) {
        return Ok(None);
    }
    credential_params(&deployment.litellm_params).map(Some)
}

pub fn provider_credentials(
    catalog: &DeploymentCatalog,
    model_id: &str,
    team_id: Option<&str>,
    stored: &BTreeMap<String, JsonObject>,
) -> Result<Option<JsonObject>, CredentialError> {
    let Some(deployment) = credential_deployment(catalog, model_id, team_id) else {
        return Ok(None);
    };
    let params = &deployment.litellm_params;
    let mut credentials = credential_params(params)?;
    if let Some(Json::Text(name)) = params.get("litellm_credential_name") {
        if let Some(values) = stored.get(name).filter(|entry| !entry.is_empty()) {
            credentials.extend(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        } else {
            tracing::warn!(
                credential = %name,
                "a deployment names a stored credential that the credential list does not hold"
            );
        }
    }
    let model = match params.get("model") {
        Some(Json::Text(model)) => model.clone(),
        _ => return Err(CredentialError::MissingModel),
    };
    let provider = match params.get("custom_llm_provider") {
        Some(Json::Text(declared)) => declared.clone(),
        _ => model.split_once('/').map_or_else(
            || String::from("openai"),
            |(prefix, _rest)| prefix.to_owned(),
        ),
    };
    credentials.insert("model".to_owned(), Json::Text(model));
    credentials.insert("custom_llm_provider".to_owned(), Json::Text(provider));
    Ok(Some(credentials))
}

pub fn sensitive_credential_hash(litellm_params: &JsonObject) -> Result<String, CredentialError> {
    let credentials = credential_params(litellm_params)?;
    let encoded = serde_json::to_vec(&Json::Object(credentials))
        .map_err(|source| CredentialError::Encode { source })?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
