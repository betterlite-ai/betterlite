#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::{Helper as _, HelperRun, run_case};

use super::Op;

#[rstest]
fn replays_every_datadog_llm_metrics_case_against_litellm(
    #[files("cases/helpers/datadog-llm-metrics/*.yaml")] path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("the case is readable");
    let case = serde_saphyr::from_str::<HelperCase>(&source).expect("the case parses");
    validate_helper(&path, &case).expect("the case validates");
    assert_eq!(case.call.operation, "b3937");
    let run = run_case(&case, false).expect("the case runs");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[test]
fn an_unknown_function_is_not_a_b3937_input() {
    let input = serde_json::json!({ "function": "nope" });
    assert!(Op::check(&input).is_err());
}
