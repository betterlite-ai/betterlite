pub(crate) mod scenario;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model_group: String,
    pub deployments: Vec<DeploymentInput>,
    #[serde(default)]
    pub counters: BTreeMap<String, CounterInput>,
    #[serde(default)]
    pub local: BTreeMap<String, i64>,
    pub steps: Vec<Step>,
}

// one deployment dict of the pinned model_list, its limits in the three sources the selector reads
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub params_tpm: Option<Json>,
    #[serde(default)]
    pub params_rpm: Option<Json>,
    #[serde(default)]
    pub info_tpm: Option<Json>,
    #[serde(default)]
    pub info_rpm: Option<Json>,
    #[serde(default)]
    pub tpm: Option<Json>,
    #[serde(default)]
    pub rpm: Option<Json>,
}

// the counter values the shared store holds for one deployment, whatever window key reads them
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CounterInput {
    #[serde(default)]
    pub tpm: Option<i64>,
    #[serde(default)]
    pub rpm: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    #[serde(default)]
    pub now_millis: u64,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub request: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Arm,
    SelectAsync,
    SelectSync,
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
    pub reads: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub kind: StepKind,
    pub armed: Option<bool>,
    pub selected: Option<String>,
    pub error: Option<ErrorShape>,
}

// the fields of the raised error, absent where the facade raises one that declares none
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorShape {
    pub status_code: Option<u16>,
    pub code: Option<String>,
    pub error_type: Option<String>,
    pub retry_after: Option<String>,
    pub message: String,
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    match scenario::run(input, random) {
        Ok(output) => Ok(ProbeOutput::UsageRoutingV2(output)),
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
    }
}
