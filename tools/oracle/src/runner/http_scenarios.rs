use self::fixtures::run::run_case;
use crate::OracleError;
use crate::cli::Options;
use crate::http_scenarios::model::HttpScenario;
use crate::paths::artifact_root;

mod fixtures;

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
