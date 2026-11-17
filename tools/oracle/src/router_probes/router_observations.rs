use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use betterlite_core::api::responses_usage::{Usage, UsageInput, normalize_usage};
use betterlite_router::router_observations::RouterObservations;
use betterlite_router::router_observations::failure::{AsyncFailureEvent, FailureEvent};
use betterlite_router::router_observations::keys::{outcome_keys, usage_keys};
use betterlite_router::router_observations::success::{
    ObservationMetadata, RequestFacts, ResponseEvent, SuccessEvent,
};
use betterlite_router::router_observations::usage::DeploymentUsage;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod input;
mod stores;

use self::input::{FailureInput, Payload, Step, deployment_usage, failure_facts, retry_after};
use self::stores::{ProbeCooldowns, ProbeCounters};

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub key: Option<String>,
    pub cooldown: Option<bool>,
    pub successes: i64,
    pub failures: i64,
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    // the cooldown cache's own float, written the way the Python probe formats it
    pub cooldown_time: Option<String>,
}

struct Scenario<'a> {
    payload: &'a Payload,
    counters: ProbeCounters,
    cooldowns: ProbeCooldowns,
    request: RequestFacts<'a>,
    usage: DeploymentUsage,
    now: DateTime<Utc>,
}

impl<'a> Scenario<'a> {
    fn new(payload: &'a Payload) -> Result<Self, OperationError> {
        let now = DateTime::from_timestamp_millis(payload.now_millis)
            .ok_or_else(|| invalid("the probe clock is out of range"))?;
        Ok(Self {
            payload,
            counters: ProbeCounters::default(),
            cooldowns: ProbeCooldowns::new(payload),
            request: RequestFacts {
                model_group: Some(&payload.catalog.model_group),
                deployment_name: &payload.catalog.model,
                deployment_id: Some(&payload.catalog.target_id),
            },
            usage: deployment_usage(&payload.catalog.limits),
            now,
        })
    }

    fn observations(&self) -> RouterObservations<'_> {
        RouterObservations {
            counters: &self.counters,
            cooldowns: &self.cooldowns,
            cooldown_time: self.payload.router.cooldown_time,
        }
    }

    fn run_step(
        &self,
        metadata: &mut ObservationMetadata,
        step: &Step,
    ) -> Result<(Option<String>, Option<bool>), OperationError> {
        match step {
            Step::Success {
                call_type,
                standard_logging_object,
                total_tokens,
            } => {
                let total = standard_logging_object.then_some(*total_tokens);
                Ok((self.success(call_type.as_deref(), total, metadata), None))
            }
            Step::Response { total_tokens } => {
                self.response(*total_tokens, metadata)?;
                Ok((None, None))
            }
            Step::Failure { failure } => Ok((None, Some(self.failure(failure)?))),
            Step::AsyncFailure { call_type } => {
                self.async_failure(call_type.as_deref())?;
                Ok((None, None))
            }
        }
    }

    fn success(
        &self,
        call_type: Option<&str>,
        total_tokens: Option<i64>,
        metadata: &ObservationMetadata,
    ) -> Option<String> {
        let event = SuccessEvent {
            call_type,
            total_tokens,
            request: self.request,
            deployment: Some(&self.usage),
        };
        self.observations().on_success(&event, metadata, self.now)
    }

    fn response(
        &self,
        total_tokens: i64,
        metadata: &mut ObservationMetadata,
    ) -> Result<(), OperationError> {
        let usage = usage_with(total_tokens)?;
        let event = ResponseEvent {
            usage: &usage,
            request: self.request,
            deployment: Some(&self.usage),
        };
        // LiteLLM's increment_deployment_usage_for_response reports nothing back to its caller
        let _counted = self
            .observations()
            .count_response_usage(&event, metadata, self.now);
        Ok(())
    }

    fn failure(&self, failure: &FailureInput) -> Result<bool, OperationError> {
        let facts = failure_facts(failure);
        let event = FailureEvent {
            deployment_id: Some(&self.payload.catalog.target_id),
            failure: &facts,
            requested_model_group: Some(&self.payload.catalog.model_group),
            model_info_cooldown_time: self.payload.deployment.info_cooldown_time,
            litellm_params_cooldown_time: self.payload.deployment.params_cooldown_time,
            retry_after: retry_after(failure),
        };
        self.observations()
            .on_failure(&event, self.payload.now_millis, self.now)
            .map_err(|error| invalid(&error.to_string()))
    }

    fn async_failure(&self, call_type: Option<&str>) -> Result<(), OperationError> {
        let event = AsyncFailureEvent {
            call_type,
            model_group: Some(&self.payload.catalog.model_group),
            deployment_name: &self.payload.catalog.model,
            deployment_id: Some(&self.payload.catalog.target_id),
        };
        self.observations()
            .on_async_failure(&event, self.now)
            .map_err(|error| invalid(&error.to_string()))
    }

    fn observe(&self, key: Option<String>, cooldown: Option<bool>) -> StepOutput {
        let target = self.payload.catalog.target_id.as_str();
        let outcome = outcome_keys(target);
        let (tpm, rpm) = self.group_usage();
        StepOutput {
            key,
            cooldown,
            successes: self.counters.value(&outcome.successes).unwrap_or_default(),
            failures: self.counters.value(&outcome.failures).unwrap_or_default(),
            tpm,
            rpm,
            cooldown_time: self
                .cooldowns
                .active_time(target)
                .map(|seconds| format!("{:.6}", seconds.0)),
        }
    }

    // Router.get_model_group_usage: the int counters of every deployment in the group, summed per limit
    fn group_usage(&self) -> (Option<i64>, Option<i64>) {
        let mut tpm: Option<i64> = None;
        let mut rpm: Option<i64> = None;
        for index in 0..self.payload.catalog.group_size {
            let id = group_id(&self.payload.catalog.target_id, index);
            let keys = usage_keys(&id, &self.payload.catalog.model, self.now);
            if let Some(value) = self.counters.value(&keys.tpm) {
                tpm = Some(tpm.unwrap_or_default().saturating_add(value));
            }
            if let Some(value) = self.counters.value(&keys.rpm) {
                rpm = Some(rpm.unwrap_or_default().saturating_add(value));
            }
        }
        (tpm, rpm)
    }
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let scenario = Scenario::new(&input.0)?;
    let mut metadata = ObservationMetadata::default();
    let mut steps = Vec::with_capacity(scenario.payload.steps.len());
    for step in &scenario.payload.steps {
        let (key, cooldown) = scenario.run_step(&mut metadata, step)?;
        steps.push(scenario.observe(key, cooldown));
    }
    Ok(ProbeOutput::RouterObservations(Output { steps }))
}

fn group_id(target_id: &str, index: u32) -> String {
    if index == 0 {
        return target_id.to_owned();
    }
    format!("{target_id}-{index}")
}

fn usage_with(total_tokens: i64) -> Result<Usage, OperationError> {
    let mut usage =
        normalize_usage(UsageInput::Absent).map_err(|error| invalid(&error.to_string()))?;
    usage.total_tokens = total_tokens;
    Ok(usage)
}
