use std::collections::BTreeMap;

use betterlite_config::sections::model_visibility::Settings as ListingSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cooldown_decisions::failure::{
    ErrorStatus, FailureClass, FailureFacts, Seconds,
};
use betterlite_router::cooldown_state::health::{DeploymentHealthCache, HealthSnapshotError};
use betterlite_router::cooldown_state::settings::Settings as RouterSettings;
use betterlite_router::router_observations::RouterObservations;
use betterlite_router::router_observations::counter::CounterSink;
use betterlite_router::router_observations::failure::{
    CooldownSink, FailureError, FailureEvent, on_failure,
};
use betterlite_router::routing_value_store::Clock;

use crate::health_cycle::endpoints::endpoint_id;
use crate::health_cycle::ports::ProbeFailure;
use crate::health_cycle::state::build_deployment_health_states;

// the status LiteLLM answers when neither the live exception nor the stored row carries one
const DEFAULT_STATUS: u16 = 500;

// the pieces of the router _write_health_state_to_router_cache reads, injected so a test can record each write
pub struct RouterState<'router> {
    pub settings: &'router RouterSettings,
    pub health_states: &'router DeploymentHealthCache,
    pub counters: &'router dyn CounterSink,
    pub cooldowns: &'router dyn CooldownSink,
    pub clock: Clock,
}

#[derive(Debug, thiserror::Error)]
pub enum HealthStateError {
    #[error("the deployment health states could not be stored: {0}")]
    States(#[source] HealthSnapshotError),
    #[error("the failure of deployment {deployment_id} could not be recorded: {source}")]
    Failure {
        deployment_id: String,
        #[source]
        source: Box<FailureError>,
    },
}

// _get_endpoint_exception_status: the live exception's status first, then the integer a shared-cache reader kept
#[must_use]
pub fn exception_status(endpoint: &JsonObject, exceptions: &BTreeMap<String, ProbeFailure>) -> u16 {
    let Some(model_id) = endpoint_id(endpoint) else {
        return stored_status(endpoint);
    };
    exceptions
        .get(model_id)
        .map_or_else(|| stored_status(endpoint), |failure| failure.status_code)
}

// endpoint.get("exception_status", 500): the shared-cache row's integer, else LiteLLM's 500
fn stored_status(endpoint: &JsonObject) -> u16 {
    let Some(Json::Number(raw)) = endpoint.get("exception_status") else {
        return DEFAULT_STATUS;
    };
    let Some(code) = raw.as_u64() else {
        return DEFAULT_STATUS;
    };
    u16::try_from(code).unwrap_or(DEFAULT_STATUS)
}

// _write_health_state_to_router_cache: a failed write warns and the health cycle carries on
pub fn write_health_state(
    router: Option<&RouterState<'_>>,
    listing: ListingSettings,
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
    exceptions: &BTreeMap<String, ProbeFailure>,
) {
    let Some(router_state) = router else {
        return;
    };
    if let Err(error) = publish(router_state, listing, healthy, unhealthy, exceptions) {
        tracing::warn!("Failed to write health state to router cache: {error}");
    }
}

fn publish(
    router: &RouterState<'_>,
    listing: ListingSettings,
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
    exceptions: &BTreeMap<String, ProbeFailure>,
) -> Result<(), HealthStateError> {
    let routing = router.settings.enable_health_check_routing;
    if !routing && !listing.model_list_healthy_only {
        return Ok(());
    }
    let states = build_deployment_health_states(
        healthy,
        &effective_unhealthy(router, unhealthy, exceptions),
        &router.clock,
    );
    if !states.is_empty() {
        router
            .health_states
            .set_states(&states)
            .map_err(HealthStateError::States)?;
        let up = states.values().filter(|state| state.is_healthy).count();
        let down = states.values().filter(|state| !state.is_healthy).count();
        tracing::debug!("health_check_routing_state_updated healthy={up} unhealthy={down}");
    }
    if !routing {
        return Ok(());
    }
    cool_failures(router, unhealthy, exceptions)
}

// the state a shared-cache row keeps is its exception_status integer, so the transient filter still sees it
fn effective_unhealthy(
    router: &RouterState<'_>,
    unhealthy: &[JsonObject],
    exceptions: &BTreeMap<String, ProbeFailure>,
) -> Vec<JsonObject> {
    let mut effective = Vec::with_capacity(unhealthy.len());
    for endpoint in unhealthy {
        let status = exception_status(endpoint, exceptions);
        if router.settings.ignore_transient_errors && (status == 429 || status == 408) {
            continue;
        }
        effective.push(endpoint.clone());
    }
    effective
}

// health check routing's bookkeeping: one failure for this minute, then the cooldown policy over it
fn cool_failures(
    router: &RouterState<'_>,
    unhealthy: &[JsonObject],
    exceptions: &BTreeMap<String, ProbeFailure>,
) -> Result<(), HealthStateError> {
    let observations = RouterObservations {
        counters: router.counters,
        cooldowns: router.cooldowns,
        cooldown_time: Seconds(router.settings.default_cooldown_time),
    };
    for endpoint in unhealthy {
        let Some(model_id) = endpoint_id(endpoint) else {
            continue;
        };
        let Some(failure) = exceptions.get(model_id) else {
            continue;
        };
        let status = failure.status_code;
        if router.settings.ignore_transient_errors && (status == 429 || status == 408) {
            continue;
        }
        let _cooled = record_probe_failure(&observations, model_id, status)?;
    }
    Ok(())
}

// a probe carries only a status, not the exception class, so the facts name Unclassified
fn record_probe_failure(
    observations: &RouterObservations<'_>,
    model_id: &str,
    status: u16,
) -> Result<bool, HealthStateError> {
    let facts = FailureFacts {
        class: FailureClass::Unclassified,
        status: ErrorStatus::Code(i64::from(status)),
        advisor_orchestration: false,
        background_response_cost_poll: false,
        wrapped_exception: None,
        caller_timeout: None,
    };
    let event = FailureEvent {
        deployment_id: Some(model_id),
        failure: &facts,
        requested_model_group: None,
        model_info_cooldown_time: None,
        litellm_params_cooldown_time: None,
        retry_after: None,
    };
    on_failure(observations, &event, 0_i64).map_err(|source| HealthStateError::Failure {
        deployment_id: model_id.to_owned(),
        source: Box::new(source),
    })
}

mod tests;
