use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::least_busy::error::LeastBusyError;
use betterlite_router::least_busy::guard::InFlightCall;
use betterlite_router::least_busy::identity::{AttemptFacts, request_count_keys};
use betterlite_router::least_busy::{LeastBusy, SharedCounts};
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{RouteKey, RoutingValueStore, StoreSettings};

use super::{Input, Output, SharedState, Step, StepOutput};
use crate::router_probes::error::OperationError;

struct Scenario {
    service: Arc<LeastBusy>,
    model_group: String,
    deployments: Vec<DeploymentSpec>,
    ids: Vec<String>,
    keys: Vec<RouteKey>,
    calls: BTreeMap<u32, InFlightCall>,
    server: Option<FakeRedis>,
}

pub(super) async fn execute_scenario(input: &Input) -> Result<Output, OperationError> {
    let deployments = deployment_specs(input)?;
    let keys = request_count_keys(&input.model_group, &deployments).map_err(request_failure)?;
    let local = RoutingValueStore::new(local_settings(), Arc::new(|| 0.0))
        .map_err(|error| invalid(&error.to_string()))?;
    let (shared, server) = if input.shared {
        let server = FakeRedis::try_start()
            .await
            .map_err(|error| invalid(&error.to_string()))?;
        let store = shared_store(&server)?;
        (Some(store), Some(server))
    } else {
        (None, None)
    };
    let mut scenario = Scenario {
        service: Arc::new(LeastBusy { local, shared }),
        model_group: input.model_group.clone(),
        ids: input
            .deployments
            .iter()
            .map(|deployment| deployment.id.clone())
            .collect(),
        deployments,
        keys,
        calls: BTreeMap::new(),
        server,
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(apply_step(step, &mut scenario).await?);
    }
    Ok(Output { steps })
}

async fn apply_step(step: &Step, scenario: &mut Scenario) -> Result<StepOutput, OperationError> {
    match step {
        Step::PreCall {
            call_id,
            deployment,
            call_type,
        } => begin_call(scenario, *call_id, deployment, call_type.as_deref()).await,
        Step::Success { call_id, .. } => {
            finish_call(scenario, *call_id).await?;
            Ok(StepOutput::Success)
        }
        Step::Failure { call_id, .. } => {
            finish_call(scenario, *call_id).await?;
            Ok(StepOutput::Failure)
        }
        Step::Select => Ok(StepOutput::Select {
            selected: choose(scenario).await?,
        }),
        Step::SelectSync => Ok(StepOutput::SelectSync {
            selected: choose(scenario).await?,
        }),
        Step::Shared => shared_step(scenario).await,
        Step::Down => Ok(switch_step(scenario, true)),
        Step::Up => Ok(switch_step(scenario, false)),
    }
}

async fn begin_call(
    scenario: &mut Scenario,
    call_id: u32,
    deployment: &str,
    call_type: Option<&str>,
) -> Result<StepOutput, OperationError> {
    let facts = AttemptFacts {
        model_group: Some(&scenario.model_group),
        deployment_id: Some(deployment),
        call_type,
    };
    let started = scenario
        .service
        .pre_call(&facts)
        .await
        .map_err(request_failure)?;
    if let Some(call) = started.call {
        scenario.calls.insert(call_id, call);
    }
    Ok(StepOutput::PreCall)
}

async fn finish_call(scenario: &mut Scenario, call_id: u32) -> Result<(), OperationError> {
    let Some(call) = scenario.calls.remove(&call_id) else {
        return Err(invalid(&format!("call {call_id} was never started")));
    };
    let _released = call.release().await;
    Ok(())
}

async fn choose(scenario: &Scenario) -> Result<Option<String>, OperationError> {
    let selection = scenario
        .service
        .select(&scenario.model_group, &scenario.deployments)
        .await
        .map_err(request_failure)?;
    Ok(selection
        .index
        .and_then(|index| scenario.ids.get(index).cloned()))
}

async fn shared_step(scenario: &Scenario) -> Result<StepOutput, OperationError> {
    let (state, values) = match scenario.service.shared_counts(&scenario.keys).await {
        SharedCounts::Absent => (SharedState::Absent, Vec::new()),
        SharedCounts::Values(values) => (SharedState::Values, values),
        SharedCounts::Unreadable(_cause) => (SharedState::Unreadable, Vec::new()),
    };
    Ok(StepOutput::Shared { state, values })
}

fn switch_step(scenario: &Scenario, down: bool) -> StepOutput {
    if let Some(server) = &scenario.server {
        server.set_down(down);
    }
    if down {
        StepOutput::Down
    } else {
        StepOutput::Up
    }
}

fn deployment_specs(input: &Input) -> Result<Vec<DeploymentSpec>, OperationError> {
    input
        .deployments
        .iter()
        .map(|deployment| deployment_spec(&input.model_group, &deployment.id))
        .collect()
}

fn deployment_spec(model_group: &str, id: &str) -> Result<DeploymentSpec, OperationError> {
    let mut params = JsonObject::new();
    params.insert(
        "model".to_owned(),
        Json::Text("openai/gpt-4o-mini".to_owned()),
    );
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model_group.to_owned(),
        params,
        info,
        JsonObject::new(),
        || "generated".to_owned(),
    )
    .map_err(|error| invalid(&error.to_string()))
}

fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: None,
        default_ttl_seconds: None,
        max_size_per_item: Some(1024),
        loaded_max_size_per_item_kib: 1024,
    }
}

fn shared_store(server: &FakeRedis) -> Result<RoutingStateStore, OperationError> {
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 100,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO))
        .map_err(|error| invalid(&error.to_string()))
}

fn request_failure(error: LeastBusyError) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
