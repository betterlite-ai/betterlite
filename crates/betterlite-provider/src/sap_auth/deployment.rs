use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::error::{ConfigurationLookupError, SapAuthError};

pub const RESOURCE_GROUP_HEADER: &str = "ai-resource-group";
pub const CLIENT_TYPE_HEADER: &str = "ai-client-type";
pub const CLIENT_TYPE_VALUE: &str = "LiteLLM";
pub const ORCHESTRATION: &str = "orchestration";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    Chat,
    Embedding,
}

pub trait DeploymentConfigurations: Send + Sync {
    fn configuration(&self, configuration_id: &str) -> Result<Json, ConfigurationLookupError>;
}

pub fn orchestration_headers(
    token: &SecretString,
    resource_group: &str,
) -> Result<HeaderMap, SapAuthError> {
    let mut headers = HeaderMap::new();
    let mut authorization = HeaderValue::from_str(token.expose_secret())
        .map_err(|source| SapAuthError::InvalidHeader { source })?;
    authorization.set_sensitive(true);
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(
        RESOURCE_GROUP_HEADER,
        HeaderValue::from_str(resource_group)
            .map_err(|source| SapAuthError::InvalidHeader { source })?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        CLIENT_TYPE_HEADER,
        HeaderValue::from_static(CLIENT_TYPE_VALUE),
    );
    Ok(headers)
}

pub fn pick_orchestration_deployment(
    listing: &Json,
    mode: DeploymentMode,
    configurations: &dyn DeploymentConfigurations,
) -> Result<String, SapAuthError> {
    let Json::Object(listing_fields) = listing else {
        return Err(SapAuthError::NoOrchestrationDeployment);
    };
    let Some(Json::List(resources)) = listing_fields.get("resources") else {
        return Err(SapAuthError::NoOrchestrationDeployment);
    };
    let mut newest: Option<(String, String)> = None;
    for resource in resources {
        let Json::Object(fields) = resource else {
            return Err(SapAuthError::DeploymentEntry);
        };
        let Some((deployment_url, created_at)) = matching_deployment(fields, mode, configurations)?
        else {
            continue;
        };
        let newer = newest
            .as_ref()
            .is_none_or(|(_, newest_created)| created_at.as_str() > newest_created.as_str());
        if newer {
            newest = Some((deployment_url, created_at));
        }
    }
    newest
        .map(|(deployment_url, _)| deployment_url)
        .ok_or(SapAuthError::NoOrchestrationDeployment)
}

fn matching_deployment(
    fields: &Object,
    mode: DeploymentMode,
    configurations: &dyn DeploymentConfigurations,
) -> Result<Option<(String, String)>, SapAuthError> {
    let scenario = mode_value(fields, "scenarioId", mode)?;
    if scenario.and_then(Json::as_str) != Some(ORCHESTRATION) {
        return Ok(None);
    }
    let Some(Json::Text(configuration_id)) = fields.get("configurationId") else {
        return Err(SapAuthError::DeploymentEntry);
    };
    let configuration = configurations
        .configuration(configuration_id)
        .map_err(|source| SapAuthError::ConfigurationLookup {
            configuration_id: configuration_id.clone(),
            source,
        })?;
    let Json::Object(configuration_fields) = &configuration else {
        return Err(SapAuthError::DeploymentEntry);
    };
    let executable = mode_value(configuration_fields, "executableId", mode)?;
    if executable.and_then(Json::as_str) != Some(ORCHESTRATION) {
        return Ok(None);
    }
    let Some(Json::Text(deployment_url)) = fields.get("deploymentUrl") else {
        return Err(SapAuthError::DeploymentEntry);
    };
    let Some(Json::Text(created_at)) = fields.get("createdAt") else {
        return Err(SapAuthError::DeploymentEntry);
    };
    Ok(Some((deployment_url.clone(), created_at.clone())))
}

fn mode_value<'fields>(
    fields: &'fields Object,
    name: &str,
    mode: DeploymentMode,
) -> Result<Option<&'fields Json>, SapAuthError> {
    match mode {
        DeploymentMode::Chat => Ok(fields.get(name)),
        DeploymentMode::Embedding => fields
            .get(name)
            .map(Some)
            .ok_or(SapAuthError::DeploymentEntry),
    }
}
