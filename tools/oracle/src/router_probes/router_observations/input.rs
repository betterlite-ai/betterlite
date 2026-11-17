use serde::{Deserialize, Serialize};

use betterlite_router::cooldown_decisions::failure::{
    ErrorStatus, FailureClass, FailureFacts, Seconds, TimeoutFacts,
};
use betterlite_router::cooldown_decisions::policy::AllowedFailsPolicy;
use betterlite_router::router_observations::usage::DeploymentUsage;

use crate::router_probes::error::OperationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub now_millis: i64,
    pub catalog: Catalog,
    #[serde(default)]
    pub deployment: Deployment,
    #[serde(default)]
    pub router: RouterInput,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub model_group: String,
    pub model: String,
    pub target_id: String,
    #[serde(default = "default_group_size")]
    pub group_size: u32,
    #[serde(default)]
    pub alternatives: u32,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Limits {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
    pub params_tpm: Option<i64>,
    pub params_rpm: Option<i64>,
    pub params_itpm: Option<i64>,
    pub params_otpm: Option<i64>,
    pub info_tpm: Option<i64>,
    pub info_rpm: Option<i64>,
    pub info_itpm: Option<i64>,
    pub info_otpm: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Deployment {
    pub provider_default: bool,
    pub allowed_fails: Option<u32>,
    pub params_cooldown_time: Option<Seconds>,
    pub info_cooldown_time: Option<Seconds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RouterInput {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: Seconds,
    pub disable_cooldowns: bool,
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
}

impl Default for RouterInput {
    fn default() -> Self {
        Self {
            allowed_fails: 3,
            default_allowed_fails: 3,
            cooldown_time: Seconds(5.0),
            disable_cooldowns: false,
            allowed_fails_policy: None,
        }
    }
}

// a float inside a step reaches serde's content buffer as a map under serde_json's arbitrary_precision, so the step carries seconds as text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallerTimeout {
    pub client_side_timeout: bool,
    pub timeout_seconds: Option<String>,
    pub elapsed_seconds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FailureInput {
    #[serde(rename = "class")]
    pub class_name: FailureClass,
    pub status: ErrorStatus,
    #[serde(default)]
    pub advisor_orchestration: bool,
    #[serde(default)]
    pub background_response_cost_poll: bool,
    pub caller_timeout: Option<CallerTimeout>,
    pub retry_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Step {
    Success {
        call_type: Option<String>,
        #[serde(default = "default_logging_object")]
        standard_logging_object: bool,
        #[serde(default)]
        total_tokens: i64,
    },
    Response {
        total_tokens: i64,
    },
    Failure {
        failure: FailureInput,
    },
    AsyncFailure {
        call_type: Option<String>,
    },
}

const fn default_group_size() -> u32 {
    1
}

const fn default_logging_object() -> bool {
    true
}

pub(super) fn deployment_usage(limits: &Limits) -> DeploymentUsage {
    DeploymentUsage {
        tpm: limits.tpm,
        rpm: limits.rpm,
        params_tpm: limits.params_tpm,
        params_rpm: limits.params_rpm,
        model_info_tpm: limits.info_tpm,
        model_info_rpm: limits.info_rpm,
        io_token_limits: [
            limits.itpm,
            limits.otpm,
            limits.params_itpm,
            limits.params_otpm,
            limits.info_itpm,
            limits.info_otpm,
        ]
        .iter()
        .any(Option::is_some),
    }
}

pub(super) fn failure_facts(failure: &FailureInput) -> Result<FailureFacts, OperationError> {
    let caller_timeout = match failure.caller_timeout.as_ref() {
        Some(timeout) => Some(TimeoutFacts {
            client_side_timeout: timeout.client_side_timeout,
            timeout_seconds: timeout
                .timeout_seconds
                .as_deref()
                .map(seconds)
                .transpose()?,
            elapsed_seconds: timeout
                .elapsed_seconds
                .as_deref()
                .map(seconds)
                .transpose()?,
        }),
        None => None,
    };
    Ok(FailureFacts {
        class: failure.class_name,
        status: failure.status.clone(),
        advisor_orchestration: failure.advisor_orchestration,
        background_response_cost_poll: failure.background_response_cost_poll,
        wrapped_exception: None,
        caller_timeout,
    })
}

fn seconds(text: &str) -> Result<Seconds, OperationError> {
    let value = text
        .parse::<f64>()
        .map_err(|error| OperationError::InvalidRequest {
            message: format!("the caller timeout {text} is not a number of seconds: {error}"),
        })?;
    Ok(Seconds(value))
}
