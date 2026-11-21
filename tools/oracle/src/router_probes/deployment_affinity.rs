use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::identity::AffinityFlag;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::deployment_affinity::{
    DeploymentAffinity, FilterRequest, PreCallRequest, PriorityGroups, Settings,
};
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_value_store::{Clock, StoreSettings};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model: String,
    pub deployments: Vec<Deployment>,
    pub steps: Vec<Step>,
    pub ttl_seconds: u32,
    #[serde(default)]
    pub user_key: bool,
    #[serde(default)]
    pub responses: bool,
    #[serde(default)]
    pub session: bool,
    #[serde(default)]
    pub model_group_affinity_config: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub priority_groups: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub pool: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<JsonObject>,
    #[serde(default)]
    pub litellm_metadata: Option<JsonObject>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub target_order: Option<i64>,
    #[serde(default)]
    pub model_info: Option<JsonObject>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Filter,
    Hook,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub action: StepKind,
    pub selected: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observation>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime
        .block_on(execute(input))
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(ProbeOutput::DeploymentAffinity(output))
}

async fn execute(input: &Input) -> Result<Output, OperationError> {
    let now = Arc::new(Mutex::new(0.0_f64));
    let handle = Arc::clone(&now);
    let clock: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    let local =
        LocalPins::new(local_settings(), clock).map_err(|error| invalid(&error.to_string()))?;
    let mut defaults = BTreeSet::new();
    if input.user_key {
        defaults.insert(AffinityFlag::Deployment);
    }
    if input.responses {
        defaults.insert(AffinityFlag::ResponsesApi);
    }
    if input.session {
        defaults.insert(AffinityFlag::Session);
    }
    let config = if input.model_group_affinity_config.is_empty() {
        None
    } else {
        Some(input.model_group_affinity_config.clone())
    };
    let names = input.priority_groups.clone();
    let predicate: PriorityGroups =
        Arc::new(move |model: &str| names.iter().any(|group| group == model));
    let service = DeploymentAffinity::new(
        AffinityClaims {
            local,
            shared: None,
        },
        Settings {
            ttl_seconds: input.ttl_seconds,
            defaults,
            model_group_affinity_config: config,
        },
        Some(predicate),
    );
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        *now.lock().unwrap_or_else(PoisonError::into_inner) = seconds(step.now_millis);
        steps.push(apply_step(&service, input, step).await?);
    }
    Ok(Output { steps })
}

async fn apply_step(
    service: &DeploymentAffinity,
    input: &Input,
    step: &Step,
) -> Result<Observation, OperationError> {
    match step.kind {
        StepKind::Filter => filter_step(service, input, step).await,
        StepKind::Hook => hook_step(service, step).await,
    }
}

async fn filter_step(
    service: &DeploymentAffinity,
    input: &Input,
    step: &Step,
) -> Result<Observation, OperationError> {
    let deployments = pool(input, step)?;
    let kwargs = request_kwargs(step);
    let model = step.model.as_deref().unwrap_or(&input.model);
    let selected = service
        .filter_deployments(&FilterRequest {
            model,
            deployments: &deployments,
            request_kwargs: Some(&kwargs),
            target_order: step.target_order,
        })
        .await
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(Observation {
        action: StepKind::Filter,
        selected: selected
            .iter()
            .filter_map(|deployment| deployment.id().map(str::to_owned))
            .collect(),
    })
}

async fn hook_step(
    service: &DeploymentAffinity,
    step: &Step,
) -> Result<Observation, OperationError> {
    let kwargs = hook_kwargs(step);
    service
        .pre_call_deployment_hook(&PreCallRequest {
            request_kwargs: &kwargs,
            model_info: step.model_info.as_ref(),
        })
        .await
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(Observation {
        action: StepKind::Hook,
        selected: Vec::new(),
    })
}

fn pool(input: &Input, step: &Step) -> Result<Vec<DeploymentSpec>, OperationError> {
    let mut deployments = Vec::new();
    for deployment in &input.deployments {
        let wanted = step
            .pool
            .as_ref()
            .is_none_or(|ids| ids.contains(&deployment.id));
        if wanted {
            deployments.push(
                deployment_spec(&input.model, deployment)
                    .map_err(|error| invalid(&error.to_string()))?,
            );
        }
    }
    Ok(deployments)
}

// the filter's request arguments: both metadata channels, the prior response id and the retry order
fn request_kwargs(step: &Step) -> JsonObject {
    let mut kwargs = JsonObject::new();
    if let Some(metadata) = &step.metadata {
        kwargs.insert("metadata".to_owned(), Json::Object(metadata.clone()));
    }
    if let Some(metadata) = &step.litellm_metadata {
        kwargs.insert(
            "litellm_metadata".to_owned(),
            Json::Object(metadata.clone()),
        );
    }
    if let Some(previous) = &step.previous_response_id {
        kwargs.insert(
            "previous_response_id".to_owned(),
            Json::Text(previous.clone()),
        );
    }
    if let Some(target_order) = step.target_order {
        kwargs.insert(
            "_target_order".to_owned(),
            Json::Number(Number::from(target_order)),
        );
    }
    kwargs
}

// the hook's arguments add the model_info the router attaches after selecting a deployment
fn hook_kwargs(step: &Step) -> JsonObject {
    let mut kwargs = request_kwargs(step);
    if let Some(model_info) = &step.model_info {
        kwargs.insert("model_info".to_owned(), Json::Object(model_info.clone()));
    }
    kwargs
}

fn deployment_spec(
    model_group: &str,
    deployment: &Deployment,
) -> Result<DeploymentSpec, InvalidDeployment> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(deployment.model.clone()));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(deployment.id.clone()));
    DeploymentSpec::new(
        model_group.to_owned(),
        params,
        info,
        IndexMap::new(),
        String::new,
    )
}

fn seconds(now_millis: u64) -> f64 {
    Number::from(now_millis)
        .as_f64()
        .map_or(0.0, |millis| millis / 1000.0)
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
