mod scenario;

use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model_group: String,
    pub shared: bool,
    pub deployments: Vec<Deployment>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    PreCall {
        call_id: u32,
        deployment: String,
        #[serde(default)]
        call_type: Option<String>,
    },
    Success {
        call_id: u32,
        callback: Callback,
    },
    Failure {
        call_id: u32,
        callback: Callback,
    },
    Select,
    SelectSync,
    Shared,
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Callback {
    Sync,
    Async,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepOutput {
    PreCall,
    Success,
    Failure,
    Select {
        selected: Option<String>,
    },
    SelectSync {
        selected: Option<String>,
    },
    Shared {
        state: SharedState,
        values: Vec<Option<i64>>,
    },
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedState {
    Values,
    Absent,
    Unreadable,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime.block_on(scenario::execute_scenario(input))?;
    Ok(ProbeOutput::LeastBusy(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
