use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod probe;

// The strategies whose selection needs no selector state; every other one would need the router's live selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    SimpleShuffle,
    Lar1,
}

impl Strategy {
    pub(crate) const fn routing(self) -> RoutingStrategy {
        match self {
            Self::SimpleShuffle => RoutingStrategy::SimpleShuffle,
            Self::Lar1 => RoutingStrategy::Lar1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    pub use_in_pass_through: Option<bool>,
    pub weight: Option<u32>,
    pub rpm: Option<u32>,
    pub tpm: Option<u32>,
    pub order: Option<i64>,
    pub blocked: Option<bool>,
    pub supported_environments: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub model: String,
    #[serde(default)]
    pub specific_deployment: bool,
    #[serde(default)]
    pub messages: bool,
    #[serde(default)]
    pub response_format: bool,
    pub routing_strategy: Option<Strategy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub strategy: Strategy,
    #[serde(default)]
    pub enable_pre_call_checks: bool,
    #[serde(default)]
    pub drop_params: bool,
    pub models: Vec<Entry>,
    pub lookups: Vec<Lookup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    RateLimited,
    ServiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Picked {
    pub model_name: String,
    pub model: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Option<Picked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub lookups: Vec<Found>,
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    probe::run(input, random)
}
