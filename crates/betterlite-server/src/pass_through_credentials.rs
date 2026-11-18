use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct VertexPassThroughCredentials {
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassThroughDeployment {
    pub model_name: Option<String>,
    pub litellm_params: JsonObject,
}

/// Supplies the Router model list and the two LiteLLM-owned resolution seams.
pub trait PassThroughCredentialSource: Send + Sync {
    fn deployments(&self) -> Vec<PassThroughDeployment>;
    fn provider_for(&self, model: &str, custom_provider: Option<&str>) -> Option<String>;
    fn credential_values(&self, name: &str) -> Option<JsonObject>;
}

pub struct PassthroughEndpointRouter {
    source: Option<Arc<dyn PassThroughCredentialSource>>,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub deployment_key_to_vertex_credentials: BTreeMap<String, VertexPassThroughCredentials>,
    pub default_vertex_config: Option<VertexPassThroughCredentials>,
}

impl PassthroughEndpointRouter {
    #[must_use]
    pub fn new(
        source: Option<Arc<dyn PassThroughCredentialSource>>,
        environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    ) -> Self {
        Self {
            source,
            environment,
            deployment_key_to_vertex_credentials: BTreeMap::new(),
            default_vertex_config: None,
        }
    }

    #[must_use]
    pub fn get_credentials(
        &self,
        custom_llm_provider: &str,
        region_name: Option<&str>,
    ) -> Option<String> {
        if let Some(source) = &self.source {
            for deployment in source.deployments() {
                let params = &deployment.litellm_params;
                if params.get("use_in_pass_through") != Some(&Json::Bool(true)) {
                    continue;
                }
                let Some(Json::Text(model)) = params.get("model") else {
                    continue;
                };
                if source
                    .provider_for(model, text(params, "custom_llm_provider"))
                    .as_deref()
                    != Some(custom_llm_provider)
                {
                    continue;
                }
                let credentials = credential_values(source.as_ref(), params);
                let api_base =
                    str_value(&credentials, "api_base").or_else(|| text(params, "api_base"));
                if Self::_get_region_name_from_api_base(custom_llm_provider, api_base)
                    != region_name
                {
                    continue;
                }
                if let Some(key) =
                    str_value(&credentials, "api_key").or_else(|| text(params, "api_key"))
                {
                    return Some(key.to_owned());
                }
                // A matching deployment without a key contributes no result; later matching
                // deployments remain eligible, while provider and region filters still apply.
            }
        }
        self.environment
            .lookup(&Self::_get_default_env_variable_name_passthrough_endpoint(
                custom_llm_provider,
            ))
    }

    #[must_use]
    pub fn get_vertex_credentials_from_router_deployments(
        &self,
        model: Option<&str>,
    ) -> Option<VertexPassThroughCredentials> {
        let source = self.source.as_ref()?;
        let resolved = source
            .deployments()
            .into_iter()
            .filter_map(|deployment| {
                let params = &deployment.litellm_params;
                if params.get("use_in_pass_through") != Some(&Json::Bool(true)) {
                    return None;
                }
                let upstream = text(params, "model")?;
                if source
                    .provider_for(upstream, text(params, "custom_llm_provider"))
                    .as_deref()
                    != Some("vertex_ai")
                {
                    return None;
                }
                let values = credential_values(source.as_ref(), params);
                let project = str_value(&values, "vertex_project")
                    .or_else(|| text(params, "vertex_project"))?;
                let location = str_value(&values, "vertex_location")
                    .or_else(|| text(params, "vertex_location"))?;
                let stored = values
                    .get("vertex_credentials")
                    .filter(|value| matches!(value, Json::Text(_) | Json::Object(_)));
                let vertex_credentials = stored.cloned().or_else(|| {
                    params
                        .get("vertex_credentials")
                        .filter(|value| matches!(value, Json::Text(_) | Json::Object(_)))
                        .cloned()
                });
                Some((
                    deployment,
                    VertexPassThroughCredentials {
                        vertex_project: Some(project.to_owned()),
                        vertex_location: Some(location.to_owned()),
                        vertex_credentials,
                    },
                ))
            })
            .collect::<Vec<_>>();
        if let Some(model) = model {
            if let Some((_, credentials)) = resolved
                .iter()
                .find(|(deployment, _)| deployment_matches_model(deployment, model))
            {
                return Some(credentials.clone());
            }
        }
        let identities = resolved
            .iter()
            .map(|(_, value)| {
                (
                    value.vertex_project.clone(),
                    value.vertex_location.clone(),
                    credential_identity(value.vertex_credentials.as_ref()),
                )
            })
            .collect::<BTreeSet<_>>();
        (identities.len() == 1)
            .then(|| resolved.first().map(|(_, value)| value.clone()))
            .flatten()
    }

    #[must_use]
    pub fn get_vertex_credentials(
        &self,
        project_id: Option<&str>,
        location: Option<&str>,
    ) -> Option<VertexPassThroughCredentials> {
        let key = Self::_get_deployment_key(project_id, location);
        key.as_ref()
            .and_then(|key| self.deployment_key_to_vertex_credentials.get(key))
            .or(self.default_vertex_config.as_ref())
            .cloned()
    }

    pub fn set_default_vertex_config(&mut self, config: Option<JsonObject>) {
        self.default_vertex_config = Some(match config {
            None => VertexPassThroughCredentials {
                vertex_project: self.environment.lookup("DEFAULT_VERTEXAI_PROJECT"),
                vertex_location: self.environment.lookup("DEFAULT_VERTEXAI_LOCATION"),
                vertex_credentials: self
                    .environment
                    .lookup("DEFAULT_GOOGLE_APPLICATION_CREDENTIALS")
                    .map(Json::Text),
            },
            Some(mut values) => {
                for value in values.values_mut() {
                    if let Json::Text(reference) = value {
                        if let Some(name) = reference.strip_prefix("os.environ/") {
                            *value = self.environment.lookup(name).map_or(Json::Null, Json::Text);
                        }
                    }
                }
                VertexPassThroughCredentials {
                    vertex_project: json_text(values.get("vertex_project")),
                    vertex_location: json_text(values.get("vertex_location")),
                    vertex_credentials: match values.get("vertex_credentials") {
                        Some(Json::Text(_) | Json::Object(_)) => {
                            values.get("vertex_credentials").cloned()
                        }
                        _ => None,
                    },
                }
            }
        });
    }

    pub fn add_vertex_credentials(
        &mut self,
        project_id: &str,
        location: &str,
        vertex_credentials: Option<Json>,
    ) {
        self.deployment_key_to_vertex_credentials.insert(
            format!("{project_id}-{location}"),
            VertexPassThroughCredentials {
                vertex_project: Some(project_id.to_owned()),
                vertex_location: Some(location.to_owned()),
                vertex_credentials,
            },
        );
    }

    #[must_use]
    pub fn _get_deployment_key(project_id: Option<&str>, location: Option<&str>) -> Option<String> {
        Some(format!("{}-{}", project_id?, location?))
    }

    #[must_use]
    pub fn _get_region_name_from_api_base(
        custom_llm_provider: &str,
        api_base: Option<&str>,
    ) -> Option<&'static str> {
        (custom_llm_provider == "assemblyai" && api_base.is_some_and(|base| base.contains("eu")))
            .then_some("eu")
    }

    #[must_use]
    pub fn _get_default_env_variable_name_passthrough_endpoint(
        custom_llm_provider: &str,
    ) -> String {
        format!("{}_API_KEY", custom_llm_provider.to_uppercase())
    }
}

fn credential_values(source: &dyn PassThroughCredentialSource, params: &JsonObject) -> JsonObject {
    text(params, "litellm_credential_name")
        .and_then(|name| source.credential_values(name))
        .unwrap_or_default()
}

fn text<'a>(values: &'a JsonObject, key: &str) -> Option<&'a str> {
    json_text(values.get(key))
}
fn json_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}
fn str_value<'a>(values: &'a JsonObject, key: &str) -> Option<&'a str> {
    text(values, key)
}

fn deployment_matches_model(deployment: &PassThroughDeployment, model: &str) -> bool {
    let upstream = text(&deployment.litellm_params, "model");
    deployment.model_name.as_deref() == Some(model)
        || upstream == Some(model)
        || upstream.and_then(|name| name.split_once('/').map(|(_, suffix)| suffix)) == Some(model)
}

fn credential_identity(value: Option<&Json>) -> Option<String> {
    fn canonical(value: &Json) -> serde_json::Value {
        match value {
            Json::Null => serde_json::Value::Null,
            Json::Bool(value) => serde_json::Value::Bool(*value),
            Json::Number(value) => serde_json::Value::Number(value.clone()),
            Json::Text(value) => serde_json::Value::String(value.clone()),
            Json::List(values) => serde_json::Value::Array(values.iter().map(canonical).collect()),
            Json::Object(values) => {
                let sorted = values
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect::<BTreeMap<_, _>>();
                serde_json::Value::Object(sorted.into_iter().collect())
            }
        }
    }
    value.map(|value| match value {
        Json::Text(text) => text.clone(),
        Json::Object(_) => canonical(value).to_string(),
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests;
