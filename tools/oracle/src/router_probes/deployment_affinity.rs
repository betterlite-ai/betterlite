use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod engine;

// the probe operation enum keeps one pointer for this payload instead of every field inline
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    model: String,
    deployments: Vec<Deployment>,
    steps: Vec<Step>,
    ttl_seconds: u32,
    #[serde(default)]
    defaults: Vec<DefaultFlag>,
    #[serde(default)]
    model_group_affinity_config: IndexMap<String, Vec<String>>,
    #[serde(default)]
    priority_groups: Vec<String>,
}

// the three optional_pre_call_checks entries Router turns into a global affinity flag
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum DefaultFlag {
    UserKey,
    Responses,
    Session,
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
        .block_on(engine::execute(input.0.as_ref()))
        .map_err(|error| engine::invalid(&error.to_string()))?;
    Ok(ProbeOutput::DeploymentAffinity(output))
}
