use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;
use crate::router_probes::single_candidate::Deployment;

mod scenarios;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    Validate,
    Reinit,
    Override,
    Dispatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub function: Function,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub next_strategy: Option<String>,
    #[serde(default)]
    pub routing_strategy: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub deployments: Vec<Deployment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub selector: Option<String>,
    pub callbacks: Vec<String>,
    pub input_callbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum Output {
    Validate {
        valid: bool,
    },
    Reinit {
        before: State,
        after: State,
        old_registered_after: bool,
        selector_replaced: bool,
        override_cached_before: bool,
        override_rebuilt_after: bool,
    },
    Override {
        override_strategy: Option<String>,
        ignored: bool,
        supported: Vec<String>,
        selector: Option<String>,
        selector_is_default: bool,
        selector_cached: bool,
        bound: bool,
    },
    Dispatch {
        strategy: String,
        selected: String,
    },
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let output = match input.function {
        Function::Validate => scenarios::validate(input)?,
        Function::Reinit => scenarios::reinit(input)?,
        Function::Override | Function::Dispatch => scenarios::scenario(input, random)?,
    };
    Ok(ProbeOutput::StrategyDispatch(output))
}
