use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod engine;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model: String,
    pub deployments: Vec<Deployment>,
    pub steps: Vec<Step>,
    pub ttl_seconds: u32,
    #[serde(default)]
    pub user_key: bool,
    #[serde(default)]
    pub responses: bool,
    #[serde(default)]
    pub session: bool,
    #[serde(default)]
    pub model_group_affinity_config: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub priority_groups: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub pool: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<JsonObject>,
    #[serde(default)]
    pub litellm_metadata: Option<JsonObject>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub target_order: Option<i64>,
    #[serde(default)]
    pub model_info: Option<JsonObject>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Filter,
    Hook,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub action: StepKind,
    pub selected: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observation>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| engine::invalid(&error.to_string()))?;
    let output = runtime
        .block_on(engine::execute(input))
        .map_err(|error| engine::invalid(&error.to_string()))?;
    Ok(ProbeOutput::DeploymentAffinity(output))
}
