use crate::cooldown_decisions::decision::{DeploymentFacts, MinuteSamples};
use crate::cooldown_decisions::failure::{
    FailureFacts, Seconds, is_background_cost_poll_404, is_caller_timeout_408,
};
use crate::cooldown_decisions::policy::CooldownPolicy;
use crate::cooldown_policy::error::CooldownPolicyError;
use crate::cooldown_policy::facts::{
    cooldown_event, deployment_facts, error_status_text, resolve_time_to_cooldown,
};
use crate::cooldown_policy::service::{CooldownAttempt, CooldownCall, FallbackFailure};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::DeltaStore;

// what one cooldown decision reads, owned so the async step holds no catalog borrow
pub(super) struct PreparedCooldown {
    pub(super) deployment_id: String,
    pub(super) facts: DeploymentFacts,
    pub(super) failure: FailureFacts,
    pub(super) minute: MinuteSamples,
    pub(super) time_to_cooldown: Option<Seconds>,
    pub(super) exception: String,
    pub(super) event: EventFields,
}

// router_cooldown_event_callback reads these five values off the deployment
pub(super) struct EventFields {
    pub(super) model_name: String,
    pub(super) model_id: String,
    pub(super) api_base: String,
    pub(super) provider: String,
    pub(super) exception_status: String,
}

// Router.deployment_callback_on_failure's sync half: the guards, the cooldown time and the minute failure
pub(super) fn prepare_callback<S>(
    counters: &RouterCounters<S>,
    settings: &CooldownPolicy,
    lookup: &CatalogLookup<'_>,
    attempt: &CooldownAttempt<'_>,
) -> Result<Option<PreparedCooldown>, CooldownPolicyError>
where
    S: DeltaStore + 'static,
{
    let Some(deployment) = lookup.get_deployment(attempt.deployment_id)? else {
        return Ok(None);
    };
    let failure = attempt.failure;
    if failure.advisor_orchestration
        || is_background_cost_poll_404(&failure.status, failure.background_response_cost_poll)
        || is_caller_timeout_408(&failure.status, failure.caller_timeout.as_ref())
    {
        return Ok(None);
    }
    let time_to_cooldown = resolve_time_to_cooldown(
        deployment,
        attempt.header_retry_after,
        settings.cooldown_time,
    )?;
    let facts = deployment_facts(
        lookup,
        deployment,
        attempt.deployment_id,
        attempt.requested_model_group,
    )?;
    increment_failures(counters, attempt.deployment_id)?;
    let call = CooldownCall {
        deployment_id: attempt.deployment_id,
        failure,
        exception: attempt.exception,
        time_to_cooldown: Some(time_to_cooldown),
        requested_model_group: attempt.requested_model_group,
    };
    Ok(Some(prepare_cooldown(counters, deployment, &call, facts)))
}

// _trigger_cooldown_for_failed_deployment's sync half: the guards, the stamped id and the same precedence
pub(super) fn prepare_fallback<S>(
    counters: &RouterCounters<S>,
    settings: &CooldownPolicy,
    lookup: &CatalogLookup<'_>,
    failure: &FallbackFailure<'_>,
) -> Result<Option<PreparedCooldown>, CooldownPolicyError>
where
    S: DeltaStore + 'static,
{
    let failure_facts = failure.failure;
    if failure_facts.advisor_orchestration {
        return Ok(None);
    }
    // a generic API call's caller-supplied id can 404 on its own, so that status is not a health signal
    if failure.original_generic_function && failure_facts.status.cast_code() == 404 {
        return Ok(None);
    }
    if is_caller_timeout_408(&failure_facts.status, failure_facts.caller_timeout.as_ref()) {
        return Ok(None);
    }
    let Some(deployment_id) = failure.deployment_id else {
        return Ok(None);
    };
    let Some(deployment) = lookup.get_deployment(deployment_id)? else {
        return Ok(None);
    };
    let time_to_cooldown = resolve_time_to_cooldown(
        deployment,
        failure.header_retry_after,
        settings.cooldown_time,
    )?;
    let facts = deployment_facts(lookup, deployment, deployment_id, None)?;
    increment_failures(counters, deployment_id)?;
    let call = CooldownCall {
        deployment_id,
        failure: failure_facts,
        exception: failure.exception,
        time_to_cooldown: Some(time_to_cooldown),
        requested_model_group: None,
    };
    Ok(Some(prepare_cooldown(counters, deployment, &call, facts)))
}

pub(super) fn prepare_cooldown<S>(
    counters: &RouterCounters<S>,
    deployment: &DeploymentSpec,
    call: &CooldownCall<'_>,
    facts: DeploymentFacts,
) -> PreparedCooldown
where
    S: DeltaStore + 'static,
{
    let minute = MinuteSamples {
        successes: u64::try_from(counters.successes_for_minute(call.deployment_id)).unwrap_or(0),
        failures: u64::try_from(counters.failures_for_minute(call.deployment_id)).unwrap_or(0),
    };
    let status_text = error_status_text(&call.failure.status);
    let event = cooldown_event(deployment, call.deployment_id, &status_text);
    PreparedCooldown {
        deployment_id: call.deployment_id.to_owned(),
        facts,
        failure: call.failure.clone(),
        minute,
        time_to_cooldown: call.time_to_cooldown,
        exception: call.exception.to_owned(),
        event: EventFields {
            model_name: event.model_name.to_owned(),
            model_id: event.model_id.to_owned(),
            api_base: event.api_base.to_owned(),
            provider: event.provider.to_owned(),
            exception_status: event.exception_status.to_owned(),
        },
    }
}

fn increment_failures<S>(
    counters: &RouterCounters<S>,
    deployment_id: &str,
) -> Result<(), CooldownPolicyError>
where
    S: DeltaStore + 'static,
{
    counters
        .increment_failures_for_minute(deployment_id)
        .map_err(|source| CooldownPolicyError::Counter {
            model_id: deployment_id.to_owned(),
            source,
        })
}
