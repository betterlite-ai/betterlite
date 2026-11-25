#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

#[rstest]
fn every_server_grant_case_agrees_with_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-server-grants/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("case should be readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("case should parse");
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b4669");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}
