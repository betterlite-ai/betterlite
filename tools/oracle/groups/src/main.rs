use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use betterlite_oracle_groups::GROUPS;
use betterlite_oracle_runner::case::helper::parse_checked_helper_case;
use betterlite_oracle_runner::case::read_case_files;
use betterlite_oracle_runner::error::HelperRunError;
use betterlite_oracle_runner::helpers::{HelperRun, run_helper_cases};

fn main() -> Result<ExitCode, HelperRunError> {
    let paths: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();
    let mut cases = Vec::new();
    for (path, text) in read_case_files(&paths)? {
        cases.push(parse_checked_helper_case(&path, &text, GROUPS)?);
    }
    Ok(match run_helper_cases(GROUPS, &cases, false)? {
        HelperRun::Match => ExitCode::SUCCESS,
        HelperRun::Difference => ExitCode::from(1),
    })
}
