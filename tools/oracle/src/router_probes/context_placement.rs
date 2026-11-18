use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod report;
mod steps;
mod wire;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<wire::Input>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<wire::Output>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| OperationError::InvalidRequest {
            message: format!("the probe runtime could not start: {source}"),
        })?;
    runtime
        .block_on(steps::output(&input.0))
        .map(|output| ProbeOutput::ContextPlacement(Output(Box::new(output))))
}
