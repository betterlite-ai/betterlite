use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_credentials::{
    PassThroughCredentialSource, PassThroughDeployment, PassthroughEndpointRouter,
};
use serde_json::{Value, json};

#[derive(Default)]
struct Source {
    deployments: Vec<PassThroughDeployment>,
    credentials: BTreeMap<String, JsonObject>,
}

impl PassThroughCredentialSource for Source {
    fn deployments(&self) -> Vec<PassThroughDeployment> {
        self.deployments.clone()
    }

    fn provider_for(&self, model: &str, custom_provider: Option<&str>) -> Option<String> {
        custom_provider.map(str::to_owned).or_else(|| {
            model
                .split_once('/')
                .map(|(provider, _)| provider.to_owned())
        })
    }

    fn credential_values(&self, name: &str) -> Option<JsonObject> {
        self.credentials.get(name).cloned()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let source: Arc<dyn PassThroughCredentialSource> = Arc::new(source(input));
        let environment: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(environment(input));
        let mut router = PassthroughEndpointRouter::new(Some(source), environment);
        match input.get("function").and_then(Value::as_str) {
            Some("credentials") => Ok(json!({"result": router.get_credentials(
                input.get("provider").and_then(Value::as_str).unwrap_or_default(),
                input.get("region").and_then(Value::as_str),
            )})),
            Some("vertex_deployments") => Ok(json!({"result": router
                .get_vertex_credentials_from_router_deployments(input.get("model").and_then(Value::as_str))})),
            Some("vertex_default") => {
                router.set_default_vertex_config(input.get("config").and_then(as_object));
                Ok(json!({"result": router.get_vertex_credentials(None, None)}))
            }
            Some("vertex_key") => {
                if let Some(entries) = input.get("stored").and_then(Value::as_array) {
                    for entry in entries {
                        if let (Some(project), Some(location)) = (
                            entry.get("project_id").and_then(Value::as_str),
                            entry.get("location").and_then(Value::as_str),
                        ) {
                            router.add_vertex_credentials(
                                project,
                                location,
                                entry.get("credentials").map(to_json),
                            );
                        }
                    }
                }
                Ok(json!({"result": router.get_vertex_credentials(
                    input.get("project_id").and_then(Value::as_str),
                    input.get("location").and_then(Value::as_str),
                )}))
            }
            _ => Ok(Value::Null),
        }
    }
}

fn source(input: &Value) -> Source {
    let deployments = input
        .get("deployments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|deployment| PassThroughDeployment {
            model_name: deployment
                .get("model_name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            litellm_params: deployment
                .get("litellm_params")
                .and_then(as_object)
                .unwrap_or_default(),
        })
        .collect();
    let credentials = input
        .get("credentials")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, values)| as_object(values).map(|values| (name.clone(), values)))
        .collect();
    Source {
        deployments,
        credentials,
    }
}

fn environment(input: &Value) -> BTreeMap<String, String> {
    input
        .get("environment")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| value.as_str().map(|value| (name.clone(), value.to_owned())))
        .collect()
}

fn as_object(value: &Value) -> Option<JsonObject> {
    value.as_object().map(|values| {
        values
            .iter()
            .map(|(key, value)| (key.clone(), to_json(value)))
            .collect()
    })
}

fn to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Number(value) => Json::Number(value.clone()),
        Value::String(value) => Json::Text(value.clone()),
        Value::Array(values) => Json::List(values.iter().map(to_json).collect()),
        Value::Object(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), to_json(value)))
                .collect(),
        ),
    }
}
