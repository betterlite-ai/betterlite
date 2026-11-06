use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_router::redis_failure_policy::classify::{
    Failure, FailureClass, failure_kind, is_timeout_failure,
};
use betterlite_router::redis_failure_policy::log::TimeoutLogThrottle;
use betterlite_router::redis_failure_policy::{Admission, RedisFailurePolicy, Settings, Status};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub settings: SettingsInput,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub enabled: bool,
    pub failure_threshold: u32,
    pub recovery_timeout_seconds: u64,
    pub timeout_min_duration_millis: u64,
    pub timeout_log_interval_millis: u64,
}

impl SettingsInput {
    const fn to_settings(&self) -> Settings {
        Settings {
            enabled: self.enabled,
            failure_threshold: self.failure_threshold,
            recovery_timeout: Duration::from_secs(self.recovery_timeout_seconds),
            timeout_min_duration: Duration::from_millis(self.timeout_min_duration_millis),
            timeout_log_interval: Duration::from_millis(self.timeout_log_interval_millis),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Admit {
        now_millis: u64,
        operation: String,
    },
    Failure {
        now_millis: u64,
        outer: FailureClass,
        #[serde(default)]
        causes: Vec<FailureClass>,
    },
    Finish {
        admission: usize,
    },
    Throttle {
        now_millis: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepOutput {
    Admit {
        admitted: bool,
        error: Option<String>,
        status: Status,
    },
    Failure {
        health: bool,
        timeout: bool,
        opened: bool,
        status: Status,
    },
    Finish {
        recovered: bool,
        status: Status,
    },
    Throttle {
        suppressed: Option<u32>,
    },
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let settings = input.settings.to_settings();
    let mut policy = RedisFailurePolicy::from(settings);
    let mut throttle = TimeoutLogThrottle::from(settings.timeout_log_interval);
    let mut admissions = BTreeMap::<usize, Admission>::new();
    let steps = input
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| apply(step, index, &mut policy, &mut throttle, &mut admissions))
        .collect::<Result<Vec<StepOutput>, OperationError>>()?;
    Ok(ProbeOutput::RedisFailurePolicy(Output { steps }))
}

fn apply(
    step: &Step,
    index: usize,
    policy: &mut RedisFailurePolicy,
    throttle: &mut TimeoutLogThrottle,
    admissions: &mut BTreeMap<usize, Admission>,
) -> Result<StepOutput, OperationError> {
    match step {
        Step::Admit {
            now_millis,
            operation,
        } => Ok(admit(index, *now_millis, operation, policy, admissions)),
        Step::Failure {
            now_millis,
            outer,
            causes,
        } => Ok(failure(*now_millis, *outer, causes, policy)),
        Step::Finish { admission } => finish(*admission, policy, admissions),
        Step::Throttle { now_millis } => Ok(StepOutput::Throttle {
            suppressed: throttle.admit(Duration::from_millis(*now_millis)),
        }),
    }
}

fn admit(
    index: usize,
    now_millis: u64,
    operation: &str,
    policy: &mut RedisFailurePolicy,
    admissions: &mut BTreeMap<usize, Admission>,
) -> StepOutput {
    let (admitted, error) = match policy.admit(Duration::from_millis(now_millis), operation) {
        Ok(admission) => {
            admissions.insert(index, admission);
            (true, None)
        }
        Err(error) => (false, Some(error.to_string())),
    };
    StepOutput::Admit {
        admitted,
        error,
        status: policy.status(),
    }
}

fn failure(
    now_millis: u64,
    outer: FailureClass,
    causes: &[FailureClass],
    policy: &mut RedisFailurePolicy,
) -> StepOutput {
    let opened = policy
        .record_failure(
            Duration::from_millis(now_millis),
            Failure {
                class: outer,
                causes,
            },
        )
        .is_some();
    StepOutput::Failure {
        health: failure_kind(outer, causes).is_some(),
        timeout: is_timeout_failure(outer, causes),
        opened,
        status: policy.status(),
    }
}

fn finish(
    admission: usize,
    policy: &mut RedisFailurePolicy,
    admissions: &BTreeMap<usize, Admission>,
) -> Result<StepOutput, OperationError> {
    let Some(scope) = admissions.get(&admission) else {
        return Err(OperationError::InvalidRequest {
            message: "the admission index is not an admitted step".to_owned(),
        });
    };
    let recovered = policy.finish(*scope).is_some();
    Ok(StepOutput::Finish {
        recovered,
        status: policy.status(),
    })
}
