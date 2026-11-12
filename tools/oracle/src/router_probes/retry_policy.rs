use std::collections::BTreeMap;

use betterlite_router::cooldown_decisions::failure::FailureClass;
use betterlite_router::retry_policy::backoff::{
    BackoffSettings, DEFAULT_INITIAL_RETRY_DELAY, DEFAULT_JITTER, DEFAULT_MAX_RETRY_DELAY,
    RetryTiming, sleep_before_retry,
};
use betterlite_router::retry_policy::eligibility::{Availability, RetryDecision, decide};
use betterlite_router::retry_policy::policy::{RetryCounts, RetryPolicies, RetryPolicy, plan};
use betterlite_router::retry_policy::retry_after::parse_retry_after;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub function: Function,
    #[serde(default)]
    pub count: Option<CountInput>,
    #[serde(default)]
    pub eligibility: Option<EligibilityInput>,
    #[serde(default)]
    pub sleep: Option<SleepInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    Count,
    Reset,
    Eligibility,
    Sleep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountInput {
    pub error: ErrorInput,
    pub model_group: Option<String>,
    #[serde(default)]
    pub retry_policy: Option<RetryPolicy>,
    #[serde(default)]
    pub model_group_retry_policy: Option<BTreeMap<String, RetryPolicy>>,
    pub request_num_retries: Option<i64>,
    pub router_num_retries: i64,
    pub deployment_num_retries: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorInput {
    pub class: FailureClass,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilityInput {
    pub steps: Vec<EligibilityStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EligibilityStep {
    pub error: ErrorInput,
    pub healthy_deployments: usize,
    pub all_deployments: usize,
    #[serde(default)]
    pub context_window_fallbacks: Option<Vec<String>>,
    #[serde(default)]
    pub content_policy_fallbacks: Option<Vec<String>>,
    #[serde(default)]
    pub regular_fallbacks: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SleepInput {
    pub now_unix_millis: i64,
    pub steps: Vec<SleepStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SleepStep {
    pub healthy_deployments: usize,
    pub all_deployments: usize,
    pub retry_after_header: Option<String>,
    pub remaining_retries: i64,
    pub max_retries: i64,
    pub min_timeout: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub num_retries: Option<i64>,
    pub policy_applies: Option<bool>,
    pub policy: Option<RetryPolicy>,
    pub decisions: Option<Vec<bool>>,
    pub delays: Option<Vec<String>>,
    pub retry_afters: Option<Vec<String>>,
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Output::default();
    match payload.function {
        Function::Count => {
            let count = require(payload.count.as_ref(), "count")?;
            let outcome = plan(
                RetryCounts {
                    request: count.request_num_retries,
                    router: count.router_num_retries,
                    deployment: count.deployment_num_retries,
                },
                &policies(count),
                count.error.class,
                count.error.status,
                count.model_group.as_deref(),
            );
            fields.num_retries = Some(outcome.num_retries);
            fields.policy_applies = Some(outcome.policy_applies);
        }
        Function::Reset => fields.policy = Some(RetryPolicy::default()),
        Function::Eligibility => {
            let eligibility = require(payload.eligibility.as_ref(), "eligibility")?;
            fields.decisions = Some(
                eligibility
                    .steps
                    .iter()
                    .map(|step| {
                        matches!(
                            decide(step.error.class, step.error.status, &availability(step)),
                            RetryDecision::Retry
                        )
                    })
                    .collect(),
            );
        }
        Function::Sleep => {
            let sleep = require(payload.sleep.as_ref(), "sleep")?;
            let (delays, retry_afters) = sleep(sleep, random)?;
            fields.delays = Some(delays);
            fields.retry_afters = Some(retry_afters);
        }
    }
    Ok(ProbeOutput::RetryPolicy(fields))
}

fn policies(count: &CountInput) -> RetryPolicies {
    RetryPolicies {
        global: count.retry_policy.clone(),
        model_group: count.model_group_retry_policy.clone().unwrap_or_default(),
    }
}

fn availability(step: &EligibilityStep) -> Availability {
    Availability {
        healthy_deployments: step.healthy_deployments,
        all_deployments: step.all_deployments,
        context_window_fallbacks: step.context_window_fallbacks.as_ref().map(Vec::len),
        content_policy_fallbacks: step.content_policy_fallbacks.as_ref().map(Vec::len),
        regular_fallbacks: step.regular_fallbacks.as_ref().map(Vec::len),
    }
}

fn sleep(
    payload: &SleepInput,
    random: &mut InjectedRandom<'_>,
) -> Result<(Vec<String>, Vec<String>), OperationError> {
    let mut delays = Vec::with_capacity(payload.steps.len());
    let mut retry_afters = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        let parsed = step
            .retry_after_header
            .as_deref()
            .and_then(|header| parse_retry_after(header.as_bytes()).ok())
            .map(|value| value.seconds_until(payload.now_unix_millis));
        retry_afters.push(match parsed {
            Some(-1) | None => "invalid".to_owned(),
            Some(seconds) => format!("seconds:{seconds}"),
        });
        let settings = BackoffSettings {
            initial_retry_delay: DEFAULT_INITIAL_RETRY_DELAY,
            max_retry_delay: DEFAULT_MAX_RETRY_DELAY,
            jitter: DEFAULT_JITTER,
            min_timeout: f64::from(step.min_timeout),
        };
        // a retry sleeps only on how many deployments answer, so the fallback lists stay absent here
        let counts = Availability {
            healthy_deployments: step.healthy_deployments,
            all_deployments: step.all_deployments,
            context_window_fallbacks: None,
            content_policy_fallbacks: None,
            regular_fallbacks: None,
        };
        let delay = sleep_before_retry(
            &settings,
            RetryTiming {
                remaining_retries: step.remaining_retries,
                max_retries: step.max_retries,
            },
            &counts,
            parsed,
            random,
        );
        delays.push(format!("{delay:.6}"));
    }
    // LiteLLM raises when random() has no injected draw left, and the fallback sample must not hide that
    if random.exhausted {
        return Err(OperationError::SamplesExhausted {
            message: "the request has no sample left for this draw".to_owned(),
        });
    }
    Ok((delays, retry_afters))
}

fn require<'value, T>(value: Option<&'value T>, name: &str) -> Result<&'value T, OperationError> {
    value.ok_or_else(|| OperationError::InvalidRequest {
        message: format!("{name} is required for this function"),
    })
}
