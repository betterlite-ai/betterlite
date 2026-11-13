use self::run::run_case;
use crate::OracleError;
use crate::cli::Options;
use crate::http_scenarios::model::HttpScenario;
use crate::paths::artifact_root;

#[path = "http_scenarios/fixtures/adapters.rs"]
mod adapters;
#[path = "http_scenarios/fixtures/gateway.rs"]
mod gateway;
#[path = "http_scenarios/fixtures/run.rs"]
mod run;
pub(crate) fn run_cases(
    options: &Options,
    scenarios: &[HttpScenario],
) -> Result<bool, OracleError> {
    let root = artifact_root()?;
    let mut different = false;
    for scenario in scenarios {
        different |= run_case(options, &root, scenario)?;
    }
    Ok(different)
}
