mod cache_control;
mod compile;
mod placement;
mod support;

use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IntegrationName,
    PromptManagementGate {
        probes: Vec<support::GateProbe>,
    },
    CompileResultEmpty {
        probes: Vec<compile::Probe>,
    },
    AsyncParity {
        probes: Vec<compile::Probe>,
    },
    CacheControlCheck {
        probes: Vec<cache_control::CheckProbe>,
    },
    LoggerFactory {
        probes: Vec<cache_control::LoggerProbe>,
    },
    DelegationToPlacement {
        probes: Vec<placement::Probe>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::IntegrationName => Ok(support::integration_name()),
            Input::PromptManagementGate { probes } => support::gate(probes),
            Input::CompileResultEmpty { probes } => compile::empty(probes),
            Input::AsyncParity { probes } => compile::parity(probes),
            Input::CacheControlCheck { probes } => Ok(cache_control::check(probes)),
            Input::LoggerFactory { probes } => Ok(cache_control::logger(probes)),
            Input::DelegationToPlacement { probes } => placement::run(probes),
        }
    }
}
