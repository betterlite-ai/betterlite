#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{DeploymentParameters, PassThroughDeployment, PassthroughEndpointRouter};

fn resolve_provider(model: &str, custom_provider: Option<&str>) -> Option<String> {
    custom_provider.map(str::to_owned).or_else(|| {
        model
            .split_once('/')
            .map(|(provider, _)| provider.to_owned())
    })
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn obj(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn deployment(model_name: &str, params: JsonObject) -> PassThroughDeployment {
    PassThroughDeployment {
        model_name: model_name.to_owned(),
        litellm_params: serde_json::from_value(serde_json::json!(params)).unwrap(),
    }
}

fn available(model_name: &str, upstream: &str, extra: &[(&str, Json)]) -> PassThroughDeployment {
    let mut pairs = vec![
        ("model", text(upstream)),
        ("use_in_pass_through", Json::Bool(true)),
    ];
    pairs.extend(extra.iter().map(|(key, value)| (*key, value.clone())));
    deployment(model_name, obj(&pairs))
}

fn vertex(model_name: &str, project: &str, location: &str) -> PassThroughDeployment {
    let mut candidate = available(model_name, &format!("vertex_ai/{model_name}"), &[]);
    candidate.litellm_params.vertex_project = Some(project.to_owned());
    candidate.litellm_params.vertex_location = Some(location.to_owned());
    candidate
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn snapshot(pairs: &[(&str, JsonObject)]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        pairs
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: (*name).to_owned(),
                credential_values: values.clone(),
            })
            .collect(),
    )
}

fn router(
    deployments: Vec<PassThroughDeployment>,
    env: &[(&str, &str)],
) -> PassthroughEndpointRouter {
    PassthroughEndpointRouter {
        deployments,
        resolve_provider: Arc::new(resolve_provider),
        credentials: CredentialSnapshot::default(),
        environment: Arc::new(environment(env)),
        vertex_by_deployment_key: BTreeMap::new(),
        default_vertex_config: None,
    }
}

mod credential_accessor;
mod credential_identity;
mod credential_resolution;
mod deployment_matching;
mod vertex_credentials;
mod vertex_credentials_by_project_location;
mod vertex_default_config;
