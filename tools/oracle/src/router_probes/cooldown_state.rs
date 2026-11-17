pub(crate) mod scenario;
mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// a wall-clock second as the case spells it, compared bit for bit so both sides answer the same JSON number
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Seconds(pub f64);

impl PartialEq for Seconds {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Seconds {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub settings: SettingsInput,
    #[serde(default)]
    pub shared_cooldowns: BTreeMap<String, CooldownInput>,
    #[serde(default)]
    pub shared_health: BTreeMap<String, HealthInput>,
    #[serde(default)]
    pub deployments: Vec<DeploymentInput>,
    #[serde(default)]
    pub health: HealthSettings,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub default_cooldown_time: Seconds,
    pub redis_read_interval_seconds: Seconds,
    pub staleness_threshold: Seconds,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CooldownInput {
    pub exception_received: String,
    pub status_code: String,
    pub timestamp: Seconds,
    pub cooldown_time: Seconds,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthInput {
    #[serde(default = "default_is_healthy")]
    pub is_healthy: bool,
    #[serde(default)]
    pub timestamp: Seconds,
    #[serde(default)]
    pub reason: String,
}

const fn default_is_healthy() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub model_name: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub has_allowed_fails_policy: bool,
    #[serde(default)]
    pub scoped_groups: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Add,
    Read,
    Minimum,
    SetHealth,
    Health,
    Filter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub model_ids: Vec<String>,
    #[serde(default)]
    pub status_code: i64,
    #[serde(default)]
    pub cooldown_time: Option<Seconds>,
    #[serde(default)]
    pub exception: String,
    #[serde(default)]
    pub states: BTreeMap<String, HealthInput>,
    #[serde(default)]
    pub health_check_probe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutput {
    pub kind: StepKind,
    pub local_expiry: Option<Seconds>,
    pub local_expiries: Option<Vec<Option<Seconds>>>,
    pub active: Option<Vec<ActiveView>>,
    pub minimum: Option<Seconds>,
    pub unhealthy: Option<BTreeSet<String>>,
    pub kept: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveView {
    pub model_id: String,
    pub remaining_seconds: Seconds,
    pub exception_received: String,
    pub status_code: String,
    pub timestamp: Seconds,
    pub cooldown_time: Seconds,
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("{id:?} is not a usable deployment for this probe")]
    Deployment { id: String },
    #[error("the step names a cooldown key the router cannot use")]
    CooldownKey,
    #[error("the health snapshot write was refused")]
    HealthState,
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
        Ok(output) => Ok(ProbeOutput::CooldownState(output)),
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
    }
}
