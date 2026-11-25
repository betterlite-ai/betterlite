#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

#[rstest]
fn replays_object_permission_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/object-permissions/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("case should be readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("case should parse");
    if case.call.operation != "b4668" {
        return;
    }
    validate_helper(&path, &case).expect("case should validate");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}
