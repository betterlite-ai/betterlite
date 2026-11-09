mod fake_redis;
mod steps;

use betterlite_router::routing_value_store::{RouteIncrement, RouteValue};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub settings: SettingsInput,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub namespace: Option<String>,
    pub default_ttl: i64,
    pub password: Option<String>,
    pub failure_threshold: u32,
    pub recovery_timeout_millis: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: String,
    pub now_millis: u64,
    #[serde(default)]
    pub worker: usize,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub by: Option<RouteIncrement>,
    #[serde(default)]
    pub value: Option<RouteValue>,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub kind: String,
    pub value: Option<StepValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StepValue {
    Route(RouteValue),
    Claim {
        claimed: bool,
        current: Option<RouteValue>,
    },
    Count(i64),
    Flag(bool),
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime.block_on(steps::scenario(input))?;
    Ok(ProbeOutput::RoutingStateStore(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
