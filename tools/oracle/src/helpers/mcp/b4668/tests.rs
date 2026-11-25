#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

#[rstest]
fn replays_every_object_permission_case_against_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-object-permissions/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("case should be readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("case should parse");
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b4668");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}
