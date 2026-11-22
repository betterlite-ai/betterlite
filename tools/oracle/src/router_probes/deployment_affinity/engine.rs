use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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
use serde_json::Number;

use crate::router_probes::error::OperationError;

use super::{DefaultFlag, Deployment, Observation, Output, Payload, Step, StepKind};

pub(super) async fn execute(input: &Payload) -> Result<Output, OperationError> {
    let now = Arc::new(Mutex::new(0.0_f64));
    let service = build_service(input, &now)?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        *now.lock().unwrap_or_else(PoisonError::into_inner) = seconds(step.now_millis);
        steps.push(match step.kind {
            StepKind::Filter => filter_step(&service, input, step).await?,
            StepKind::Hook => hook_step(&service, step).await?,
        });
    }
    Ok(Output { steps })
}

fn build_service(
    input: &Payload,
    now: &Arc<Mutex<f64>>,
) -> Result<DeploymentAffinity, OperationError> {
    let handle = Arc::clone(now);
    let clock: Clock = Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
    let local =
        LocalPins::new(local_settings(), clock).map_err(|error| invalid(&error.to_string()))?;
    let mut defaults = BTreeSet::new();
    for flag in &input.defaults {
        defaults.insert(match flag {
            DefaultFlag::UserKey => AffinityFlag::Deployment,
            DefaultFlag::Responses => AffinityFlag::ResponsesApi,
            DefaultFlag::Session => AffinityFlag::Session,
        });
    }
    let config = if input.model_group_affinity_config.is_empty() {
        None
    } else {
        Some(input.model_group_affinity_config.clone())
    };
    let names = input.priority_groups.clone();
    let predicate: PriorityGroups =
        Arc::new(move |model: &str| names.iter().any(|group| group == model));
    Ok(DeploymentAffinity::new(
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
    ))
}

async fn filter_step(
    service: &DeploymentAffinity,
    input: &Payload,
    step: &Step,
) -> Result<Observation, OperationError> {
    let deployments = pool(input, step)?;
    let kwargs = request_kwargs(step);
    let model = step.model.as_deref().unwrap_or(&input.model);
    let filter = FilterRequest {
        model,
        deployments: &deployments,
        request_kwargs: Some(&kwargs),
        target_order: step.target_order,
    };
    let chosen = run_filter(service, &filter).await?;
    Ok(Observation {
        action: StepKind::Filter,
        selected: selected_ids(&chosen),
    })
}

async fn run_filter(
    service: &DeploymentAffinity,
    filter: &FilterRequest<'_>,
) -> Result<Vec<DeploymentSpec>, OperationError> {
    // the box ends the async chain here, so this crate's future layouts stay inside rustc's query depth
    Box::pin(service.filter_deployments(filter))
        .await
        .map_err(|error| invalid(&error.to_string()))
}

async fn hook_step(
    service: &DeploymentAffinity,
    step: &Step,
) -> Result<Observation, OperationError> {
    let kwargs = hook_kwargs(step);
    let request = PreCallRequest {
        request_kwargs: &kwargs,
        model_info: step.model_info.as_ref(),
    };
    Box::pin(service.pre_call_deployment_hook(&request))
        .await
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(hook_observation())
}

const fn hook_observation() -> Observation {
    Observation {
        action: StepKind::Hook,
        selected: Vec::new(),
    }
}

fn selected_ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .filter_map(|deployment| deployment.id().map(str::to_owned))
        .collect()
}

fn pool(input: &Payload, step: &Step) -> Result<Vec<DeploymentSpec>, OperationError> {
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
    Duration::from_millis(now_millis).as_secs_f64()
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

pub(super) fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
