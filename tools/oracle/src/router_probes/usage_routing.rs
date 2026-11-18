pub mod scenario;
pub mod wire;

pub type Input = wire::Input;
pub type Output = wire::Output;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    Ok(ProbeOutput::UsageRouting(scenario::run_scenario(input)?))
}
