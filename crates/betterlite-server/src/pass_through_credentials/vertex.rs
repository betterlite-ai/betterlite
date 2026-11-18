use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{
    CredentialError, PassThroughDeployment, ProviderResolver, VertexPassThroughCredentials,
    credential_values, json_text,
};

pub(super) fn resolved_vertex_deployments<'deployment>(
    deployments: &'deployment [PassThroughDeployment],
    resolve_provider: &ProviderResolver,
    credentials: &CredentialSnapshot,
) -> Vec<(
    &'deployment PassThroughDeployment,
    VertexPassThroughCredentials,
)> {
    deployments
        .iter()
        .filter_map(|deployment| {
            let record = vertex_deployment(resolve_provider, credentials, deployment)?;
            Some((deployment, record))
        })
        .collect()
}

fn vertex_deployment(
    resolve_provider: &ProviderResolver,
    credentials: &CredentialSnapshot,
    deployment: &PassThroughDeployment,
) -> Option<VertexPassThroughCredentials> {
    let params = &deployment.litellm_params;
    if !params.use_in_pass_through {
        return None;
    }
    let model = params.model.as_deref()?;
    if resolve_provider(model, params.custom_llm_provider.as_deref()).as_deref()
        != Some("vertex_ai")
    {
        return None;
    }
    let values = credential_values(credentials, params);
    let vertex_project =
        json_text(values.get("vertex_project")).or(params.vertex_project.as_deref())?;
    let vertex_location =
        json_text(values.get("vertex_location")).or(params.vertex_location.as_deref())?;
    let stored = values
        .get("vertex_credentials")
        .filter(|value| matches!(value, Json::Text(_) | Json::Object(_)));
    let vertex_credentials = stored
        .or_else(|| {
            params
                .vertex_credentials
                .as_ref()
                .filter(|value| matches!(value, Json::Text(_) | Json::Object(_)))
        })
        .cloned();
    Some(VertexPassThroughCredentials {
        vertex_project: Some(vertex_project.to_owned()),
        vertex_location: Some(vertex_location.to_owned()),
        vertex_credentials,
    })
}

pub(super) fn vertex_config_from(
    mut config: JsonObject,
    environment: &dyn EnvironmentLookup,
) -> Result<VertexPassThroughCredentials, CredentialError> {
    for (field, value) in &mut config {
        let valid = match field.as_str() {
            "vertex_project" | "vertex_location" => matches!(value, Json::Text(_) | Json::Null),
            "vertex_credentials" => matches!(value, Json::Text(_) | Json::Object(_) | Json::Null),
            _ => false,
        };
        if !valid {
            return Err(CredentialError::InvalidConfig {
                field: field.clone(),
            });
        }
        let Json::Text(reference) = value else {
            continue;
        };
        let Some(name) = reference.strip_prefix("os.environ/") else {
            continue;
        };
        *value = environment
            .lookup(name)
            .filter(|text| !text.is_empty())
            .map_or(Json::Null, Json::Text);
    }
    Ok(VertexPassThroughCredentials {
        vertex_project: json_text(config.get("vertex_project")).map(str::to_owned),
        vertex_location: json_text(config.get("vertex_location")).map(str::to_owned),
        vertex_credentials: match config.get("vertex_credentials") {
            Some(value @ (Json::Text(_) | Json::Object(_))) => Some(value.clone()),
            _ => None,
        },
    })
}
