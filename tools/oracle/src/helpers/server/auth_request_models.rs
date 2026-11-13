use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_request_models::catalog::{
    CatalogError, DeploymentRecord, ModelCatalog, RequestModelResolver,
};
use betterlite_auth::auth_request_models::sources::{
    DispatchContext, RequestSnapshot, request_dispatched_to_pass_through_endpoint,
    request_dispatched_to_provider_pass_through,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_server::auth::auth_request_models::CoreDecoders;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    route: String,
    body: Value,
    headers: Value,
    query: Value,
    team_id: Option<String>,
    deployments: Vec<DeploymentInput>,
    endpoint_marker: Option<bool>,
    path_params: Vec<String>,
    fail: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentInput {
    model_name: String,
    litellm_params: Value,
    model_info: Value,
}

struct ReadyStep {
    route: String,
    body: JsonObject,
    headers: JsonObject,
    query: JsonObject,
    team_id: Option<String>,
    deployments: Vec<DeploymentRecord>,
    endpoint_marker: Option<bool>,
    path_params: Vec<String>,
    fail: Option<String>,
}

#[derive(Clone, Default)]
struct DebugCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let steps: Vec<ReadyStep> = input
            .steps
            .iter()
            .map(ready_step)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(steps.iter().map(run_step).collect()))
    }
}

fn run_step(step: &ReadyStep) -> Value {
    let resolver = RequestModelResolver {
        catalog: Arc::new(step_catalog(step)),
        decoders: Arc::new(CoreDecoders),
    };
    let path_params: BTreeSet<String> = step.path_params.iter().cloned().collect();
    let endpoint_marker = step.endpoint_marker.map(Json::Bool);
    let snapshot = RequestSnapshot {
        route: &step.route,
        body: &step.body,
        headers: Some(&step.headers),
        query: Some(&step.query),
        dispatch: DispatchContext {
            endpoint_marker: endpoint_marker.as_ref(),
            path_params: &path_params,
        },
        team_id: step.team_id.as_deref(),
    };
    let pass_through = request_dispatched_to_pass_through_endpoint(&snapshot.dispatch);
    let provider_pass_through = request_dispatched_to_provider_pass_through(&snapshot.dispatch);
    let counter = DebugCounter::default();
    let subscriber = tracing_subscriber::registry().with(counter.clone());
    let model = with_default(subscriber, || resolver.resolve(&snapshot));
    let debug = counter.0.load(Ordering::Relaxed);
    match model {
        Ok(model) => json!({
            "ok": true,
            "value": {
                "model": model_value(&model),
                "debug": debug,
                "pass_through": pass_through,
                "provider_pass_through": provider_pass_through,
            },
        }),
        Err(_error) => json!({"ok": false}),
    }
}

fn model_value(model: &RequestedModels) -> Value {
    match model {
        RequestedModels::None => Value::Null,
        RequestedModels::One(name) => Value::String(name.clone()),
        RequestedModels::Many(names) => json!(names),
    }
}

#[derive(Debug, Default)]
struct StepCatalog {
    deployments: Vec<DeploymentRecord>,
    model_names: BTreeSet<String>,
    failing: bool,
}

impl ModelCatalog for StepCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        if model_id.is_empty() {
            return Ok(None);
        }
        if self.model_names.contains(model_id) {
            return Ok(Some(model_id.to_owned()));
        }
        if let Some(deployment) = self.deployment(model_id)? {
            if deployment.model_name.is_empty() {
                return Ok(Some(model_id.to_owned()));
            }
            return Ok(Some(deployment.model_name));
        }
        for deployment in &self.deployments {
            let Some(actual) = deployment.model.as_deref() else {
                continue;
            };
            let slash = format!("/{model_id}");
            let colon = format!(":{model_id}");
            let matched =
                actual == model_id || actual.ends_with(&slash) || actual.ends_with(&colon);
            if matched && !deployment.model_name.is_empty() {
                return Ok(Some(deployment.model_name.clone()));
            }
        }
        Ok(None)
    }

    fn deployment(
        &self,
        deployment_id: &str,
    ) -> Result<Option<DeploymentRecord>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self
            .deployments
            .iter()
            .find(|deployment| deployment.deployment_id.as_deref() == Some(deployment_id))
            .cloned())
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.model_names.clone())
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.deployments.clone())
    }
}

fn step_catalog(step: &ReadyStep) -> StepCatalog {
    let model_names = step
        .deployments
        .iter()
        .filter(|deployment| !deployment.model_name.is_empty())
        .map(|deployment| deployment.model_name.clone())
        .collect();
    StepCatalog {
        deployments: step.deployments.clone(),
        model_names,
        failing: step.fail.as_deref() == Some("resolver"),
    }
}

fn ready_step(step: &Step) -> Result<ReadyStep, Box<HelperError>> {
    Ok(ReadyStep {
        route: step.route.clone(),
        body: object(&step.body)?,
        headers: object(&step.headers)?,
        query: object(&step.query)?,
        team_id: step.team_id.clone(),
        deployments: step
            .deployments
            .iter()
            .map(ready_deployment)
            .collect::<Result<Vec<_>, _>>()?,
        endpoint_marker: step.endpoint_marker,
        path_params: step.path_params.clone(),
        fail: step.fail.clone(),
    })
}

fn ready_deployment(input: &DeploymentInput) -> Result<DeploymentRecord, Box<HelperError>> {
    let params = object(&input.litellm_params)?;
    let info = object(&input.model_info)?;
    Ok(DeploymentRecord {
        model_name: input.model_name.clone(),
        custom_llm_provider: text_field(&params, "custom_llm_provider"),
        model: text_field(&params, "model"),
        team_id: text_field(&info, "team_id"),
        deployment_id: text_field(&info, "id"),
    })
}

fn object(value: &Value) -> Result<JsonObject, Box<HelperError>> {
    serde_json::from_value(value.clone()).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn text_field(object: &JsonObject, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}
