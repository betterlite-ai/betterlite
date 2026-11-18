use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use self::vertex::{resolved_vertex_deployments, vertex_config_from};

// get_llm_provider(model, custom_llm_provider): the provider a deployment's model resolves to, none when unknown.
pub type ProviderResolver = Arc<dyn Fn(&str, Option<&str>) -> Option<String> + Send + Sync>;

// The two Router.get_model_list fields the pass-through resolver reads.
#[derive(Clone, serde::Deserialize)]
pub struct PassThroughDeployment {
    pub model_name: String,
    pub litellm_params: DeploymentParameters,
}

#[serde_with::serde_as]
#[derive(Clone, serde::Deserialize)]
pub struct DeploymentParameters {
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "serde_with::DefaultOnNull<bool>")]
    pub use_in_pass_through: bool,
    pub litellm_credential_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<Json>,
}

#[derive(Clone, Default, Eq, PartialEq, serde::Serialize)]
pub struct VertexPassThroughCredentials {
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<Json>,
}

pub struct PassthroughEndpointRouter {
    pub deployments: Vec<PassThroughDeployment>,
    pub resolve_provider: ProviderResolver,
    pub credentials: CredentialSnapshot,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub vertex_by_deployment_key: BTreeMap<String, VertexPassThroughCredentials>,
    pub default_vertex_config: Option<VertexPassThroughCredentials>,
}

impl PassthroughEndpointRouter {
    #[must_use]
    pub fn get_credentials(
        &self,
        custom_llm_provider: &str,
        region_name: Option<&str>,
    ) -> Option<SecretString> {
        if let Some(api_key) = deployment_api_key(
            &self.deployments,
            &self.resolve_provider,
            &self.credentials,
            custom_llm_provider,
            region_name,
        ) {
            return Some(api_key);
        }
        self.environment
            .lookup(&format!("{}_API_KEY", custom_llm_provider.to_uppercase()))
            .filter(|value| !value.is_empty())
            .map(SecretString::from)
    }

    #[must_use]
    pub fn vertex_deployment_credentials(
        &self,
        model: Option<&str>,
    ) -> Option<VertexPassThroughCredentials> {
        let resolved = resolved_vertex_deployments(
            &self.deployments,
            &self.resolve_provider,
            &self.credentials,
        );
        if let Some(named) = model
            && let Some((_, record)) = resolved
                .iter()
                .find(|(deployment, _)| deployment_matches_model(deployment, named))
        {
            return Some(record.clone());
        }
        let (_, first) = resolved.first()?;
        resolved
            .iter()
            .all(|(_, record)| record == first)
            .then(|| first.clone())
    }

    #[must_use]
    pub fn get_vertex_credentials(
        &self,
        project_id: Option<&str>,
        location: Option<&str>,
    ) -> Option<VertexPassThroughCredentials> {
        let (Some(named_project), Some(named_location)) = (project_id, location) else {
            return self.default_vertex_config.clone();
        };
        self.vertex_by_deployment_key
            .get(&format!("{named_project}-{named_location}"))
            .or(self.default_vertex_config.as_ref())
            .cloned()
    }

    pub fn set_default_vertex_config(
        &mut self,
        config: Option<JsonObject>,
    ) -> Result<(), CredentialError> {
        let environment = self.environment.as_ref();
        let record = match config {
            None => {
                let [vertex_project, vertex_location, credentials] = [
                    "DEFAULT_VERTEXAI_PROJECT",
                    "DEFAULT_VERTEXAI_LOCATION",
                    "DEFAULT_GOOGLE_APPLICATION_CREDENTIALS",
                ]
                .map(|name| environment.lookup(name).filter(|value| !value.is_empty()));
                VertexPassThroughCredentials {
                    vertex_project,
                    vertex_location,
                    vertex_credentials: credentials.map(Json::Text),
                }
            }
            Some(provided) => vertex_config_from(provided, environment)?,
        };
        self.default_vertex_config = Some(record);
        Ok(())
    }

    pub fn add_vertex_credentials(
        &mut self,
        project_id: &str,
        location: &str,
        vertex_credentials: Option<Json>,
    ) {
        self.vertex_by_deployment_key.insert(
            format!("{project_id}-{location}"),
            VertexPassThroughCredentials {
                vertex_project: Some(project_id.to_owned()),
                vertex_location: Some(location.to_owned()),
                vertex_credentials,
            },
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("invalid pass-through Vertex config field {field}")]
    InvalidConfig { field: String },
}

// LiteLLM only derives a region for AssemblyAI, from "eu" anywhere in the api_base. Every other provider reads none.
fn region_name_from_api_base(
    custom_llm_provider: &str,
    api_base: Option<&str>,
) -> Option<&'static str> {
    if custom_llm_provider != "assemblyai" {
        return None;
    }
    if api_base.is_some_and(|base| base.contains("eu")) {
        return Some("eu");
    }
    None
}

fn deployment_api_key(
    deployments: &[PassThroughDeployment],
    resolve_provider: &ProviderResolver,
    credentials: &CredentialSnapshot,
    custom_llm_provider: &str,
    region_name: Option<&str>,
) -> Option<SecretString> {
    for deployment in deployments {
        let params = &deployment.litellm_params;
        if !params.use_in_pass_through {
            continue;
        }
        let Some(model) = params.model.as_deref() else {
            continue;
        };
        let provider = resolve_provider(model, params.custom_llm_provider.as_deref());
        if provider.as_deref() != Some(custom_llm_provider) {
            continue;
        }
        let values = credential_values(credentials, params);
        let api_base = json_text(values.get("api_base")).or(params.api_base.as_deref());
        if region_name_from_api_base(custom_llm_provider, api_base) != region_name {
            continue;
        }
        let api_key = json_text(values.get("api_key"))
            .map(|key| SecretString::from(key.to_owned()))
            .or_else(|| params.api_key.clone());
        if api_key.is_some() {
            return api_key;
        }
    }
    None
}

fn deployment_matches_model(deployment: &PassThroughDeployment, model: &str) -> bool {
    let upstream = deployment.litellm_params.model.as_deref();
    deployment.model_name == model
        || upstream == Some(model)
        || upstream.and_then(|name| name.split_once('/').map(|(_, suffix)| suffix)) == Some(model)
}

fn credential_values(
    credentials: &CredentialSnapshot,
    params: &DeploymentParameters,
) -> JsonObject {
    params
        .litellm_credential_name
        .as_deref()
        .map_or_else(JsonObject::new, |name| credentials.values(name))
}

const fn json_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

mod tests;
mod vertex;
