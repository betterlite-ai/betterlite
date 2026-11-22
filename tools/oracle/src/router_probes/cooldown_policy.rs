mod fixtures;
mod scenario;

use betterlite_router::cooldown_decisions::failure::{FailureFacts, Seconds};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

use scenario::scenario;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub now_millis: u64,
    pub router: RouterInput,
    pub deployments: Vec<DeploymentInput>,
    pub target: String,
    pub failures: u32,
    pub failure: FailureFacts,
    pub minute: MinuteInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterInput {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: Seconds,
    pub disable_cooldowns: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: String,
    pub model_name: String,
    pub model: String,
    #[serde(default)]
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinuteInput {
    pub successes: u64,
}

// the reply buffer hands a JSON float to serde as a map under arbitrary_precision, so every float travels as fixed-point text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub cooled: Vec<bool>,
    pub events: Vec<EventOutput>,
    pub active: Vec<ActiveOutput>,
    pub failures_for_minute: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventOutput {
    pub model_name: String,
    pub model_id: String,
    pub api_base: String,
    pub provider: String,
    pub exception_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveOutput {
    pub model_id: String,
    pub status_code: String,
    pub cooldown_time: String,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| OperationError::InvalidRequest {
            message: format!("the scenario runtime could not start: {source}"),
        })?;
    let output = runtime
        .block_on(scenario(&input.0))
        .map_err(|message| OperationError::InvalidRequest { message })?;
    Ok(ProbeOutput::CooldownPolicy(output))
}
