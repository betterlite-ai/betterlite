pub(crate) mod scenario;

use std::collections::BTreeMap;
use std::io;

use betterlite_router::routing_value_store::{RouteValue, StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub cooldown: TierInput,
    #[serde(default)]
    pub remote: BTreeMap<String, RouteValue>,
    #[serde(default)]
    pub read_expiry_millis: Option<u64>,
    #[serde(default)]
    pub backfill_ttl_millis: Option<u64>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierInput {
    pub keys: Vec<String>,
    #[serde(default)]
    pub local: BTreeMap<String, RouteValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Arm,
    Read,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub request: usize,
    #[serde(default)]
    pub down: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
    pub reads: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub kind: StepKind,
    pub armed: Option<bool>,
    pub active: Option<bool>,
    pub cooldown: Option<Vec<Option<RouteValue>>>,
    pub cooldown_local: Vec<Option<RouteValue>>,
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("{key:?} is not a valid routing key")]
    InvalidKey {
        key: String,
        #[source]
        source: StoreError,
    },
    #[error("request {request} is not one of the two scenario requests")]
    UnknownRequest { request: usize },
    #[error("the time {millis} milliseconds cannot be represented as seconds")]
    TimeOverflow { millis: u64 },
    #[error("the scenario runtime could not start")]
    Runtime {
        #[source]
        source: io::Error,
    },
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    match scenario::run(input) {
        Ok(output) => Ok(ProbeOutput::RoutingReadBatch(output)),
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
    }
}
