use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::cooldown_decisions::decision::MinuteSamples;
use crate::cooldown_decisions::failure::{
    FailureFacts, Seconds, is_background_cost_poll_404, is_caller_timeout_408,
};
use crate::retry_policy::retry_after::RetryAfter;

use super::RouterObservations;
use super::counter::{CounterError, CounterSink};
use super::keys::{OutcomeKeys, ROUTER_COUNTER_TTL, outcome_keys, usage_keys};

#[derive(Debug, Clone, Copy)]
pub struct FailureEvent<'a> {
    pub deployment_id: Option<&'a str>,
    pub failure: &'a FailureFacts,
    pub requested_model_group: Option<&'a str>,
    pub model_info_cooldown_time: Option<Seconds>,
    pub litellm_params_cooldown_time: Option<Seconds>,
    pub retry_after: Option<RetryAfter>,
}

// what _set_cooldown_deployments reads once the failure was counted into this minute
#[derive(Debug, Clone, Copy)]
pub struct FailureObservation<'a> {
    pub deployment_id: &'a str,
    pub failure: &'a FailureFacts,
    pub requested_model_group: Option<&'a str>,
    pub time_to_cooldown: Seconds,
    pub minute: MinuteSamples,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CooldownError {
    #[error("the cooldown store rejected deployment {deployment_id}")]
    Rejected { deployment_id: String },
}

pub trait CooldownSink: Send + Sync {
    // Router._set_cooldown_deployments: applies the allowed-fails policy and reports whether the deployment cools down
    fn set_cooldown_deployments(
        &self,
        observation: &FailureObservation<'_>,
    ) -> Result<bool, CooldownError>;
}

#[derive(Debug, Error)]
pub enum FailureError {
    #[error("the failure counter could not be incremented")]
    Counter(#[source] CounterError),
    #[error("the cooldown decision could not be recorded")]
    Cooldown(#[source] CooldownError),
}

#[derive(Debug, Clone, Copy)]
pub struct AsyncFailureEvent<'a> {
    pub call_type: Option<&'a str>,
    pub model_group: Option<&'a str>,
    pub deployment_name: &'a str,
    pub deployment_id: Option<&'a str>,
}

// the failures that belong to another layer: an advisor run, the background cost poll, or the caller's own timeout
fn skips(failure: &FailureFacts) -> bool {
    failure.advisor_orchestration
        || is_background_cost_poll_404(&failure.status, failure.background_response_cost_poll)
        || is_caller_timeout_408(&failure.status, failure.caller_timeout.as_ref())
}

// deployment config > response header > router default, and a negative value falls through to the next source
fn cooldown_time(event: &FailureEvent<'_>, default: Seconds, now_unix_millis: i64) -> Seconds {
    // _first_present reads the model_info value even when it is negative, which hides the litellm_params one
    let configured = event
        .model_info_cooldown_time
        .or(event.litellm_params_cooldown_time)
        .filter(|value| value.0 >= 0.0);
    let header = event
        .retry_after
        .map(|value| value.seconds_until(now_unix_millis))
        .filter(|value| *value >= 0)
        .and_then(|value| Decimal::from(value).to_f64())
        .map(Seconds);
    configured.or(header).unwrap_or(default)
}

impl RouterObservations<'_> {
    pub fn on_failure(
        &self,
        event: &FailureEvent<'_>,
        now_unix_millis: i64,
        now: DateTime<Utc>,
    ) -> Result<bool, FailureError> {
        if skips(event.failure) {
            return Ok(false);
        }
        let Some(deployment_id) = event.deployment_id else {
            return Ok(false);
        };
        let time_to_cooldown = cooldown_time(event, self.cooldown_time, now_unix_millis);
        let keys = outcome_keys(deployment_id);
        self.counters
            .increment(&keys.failures, 1, ROUTER_COUNTER_TTL)
            .map_err(FailureError::Counter)?;
        let observation = FailureObservation {
            deployment_id,
            failure: event.failure,
            requested_model_group: event.requested_model_group,
            time_to_cooldown,
            minute: self.minute_samples(&keys),
        };
        self.cooldowns
            .set_cooldown_deployments(&observation)
            .map_err(FailureError::Cooldown)
    }

    pub fn on_async_failure(
        &self,
        event: &AsyncFailureEvent<'_>,
        now: DateTime<Utc>,
    ) -> Result<(), CounterError> {
        if event
            .call_type
            .is_some_and(|call_type| matches!(call_type, "retrieve_batch" | "aretrieve_batch"))
        {
            return Ok(());
        }
        // the guard needs a model group before the rpm counter is written
        let (Some(_model_group), Some(deployment_id)) = (event.model_group, event.deployment_id)
        else {
            return Ok(());
        };
        let keys = usage_keys(deployment_id, event.deployment_name, now);
        self.counters.increment(&keys.rpm, 1, ROUTER_COUNTER_TTL)
    }

    fn minute_samples(&self, keys: &OutcomeKeys) -> MinuteSamples {
        MinuteSamples {
            successes: self
                .counters
                .read(&keys.successes)
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(0),
            failures: self
                .counters
                .read(&keys.failures)
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(0),
        }
    }
}
