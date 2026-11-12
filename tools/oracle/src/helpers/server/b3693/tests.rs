#![cfg(test)]

use betterlite_oracle::case::helper::HelperCase;
use betterlite_oracle::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::{fs, path::PathBuf};

#[rstest]
fn replays_pattern_matching_oracle_cases(
    #[base_dir = "../../.."]
    #[files("cases/guardrails/pattern-matching/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
