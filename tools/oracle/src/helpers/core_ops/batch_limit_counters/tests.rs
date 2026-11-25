#![cfg(test)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::error::HelperRunError;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use thiserror::Error;

const OPERATION: &str = "batch_limit_counters";

#[derive(Debug, Error)]
enum ReplayError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot parse or validate {path}: {source}")]
    Case {
        path: PathBuf,
        #[source]
        source: HelperRunError,
    },
    #[error("cannot run {path}: {source}")]
    Run {
        path: PathBuf,
        #[source]
        source: HelperRunError,
    },
    #[error("the case names {actual} instead of {expected}")]
    Operation {
        actual: String,
        expected: &'static str,
    },
    #[error("{path} does not match LiteLLM")]
    Parity { path: PathBuf },
}

#[rstest]
fn replays_the_batch_limit_counters_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/batch-limit-counters/*.yaml")]
    path: PathBuf,
) {
    replay_case(&path).unwrap();
}

fn replay_case(path: &Path) -> Result<(), ReplayError> {
    let source = fs::read_to_string(path).map_err(|error| ReplayError::Read {
        path: path.to_owned(),
        source: error,
    })?;
    let case = parse_helper_case(path, &source).map_err(|error| ReplayError::Case {
        path: path.to_owned(),
        source: error,
    })?;
    if case.call.operation != OPERATION {
        return Err(ReplayError::Operation {
            actual: case.call.operation,
            expected: OPERATION,
        });
    }
    match run_case(&[crate::GROUP], &case, false).map_err(|error| ReplayError::Run {
        path: path.to_owned(),
        source: error,
    })? {
        HelperRun::Match => Ok(()),
        HelperRun::Difference => Err(ReplayError::Parity {
            path: path.to_owned(),
        }),
    }
}
