use betterlite_router::routing_state_store::test_scenario::{
    ScenarioInput, ScenarioOutput, run_scenario,
};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

pub type Input = ScenarioInput;
pub type Output = ScenarioOutput;

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime
        .block_on(run_scenario(input))
        .map_err(|message| invalid(&message))?;
    Ok(ProbeOutput::RoutingStateStore(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
