#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use super::Op;

#[rstest]
fn replays_every_prometheus_failure_case_against_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/prometheus-failure/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("the case is readable");
    let case = serde_saphyr::from_str::<HelperCase>(&source).expect("the case parses");
    validate_helper(&path, &case).expect("the case validates");
    assert_eq!(case.call.operation, "b3880");
    let run = run_case(&[crate::GROUP], &case, false).expect("the case runs");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[test]
fn an_unknown_function_is_not_a_b3880_input() {
    let input = serde_json::json!({ "function": "nope" });
    assert!(Op::check(&input).is_err());
}
