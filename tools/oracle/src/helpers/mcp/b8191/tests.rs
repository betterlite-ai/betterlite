#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::json;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::{Helper as _, HelperRun, run_case};

use super::Op;

fn parse_case(path: &Path) -> HelperCase {
    let source = fs::read_to_string(path).expect("case should be readable");
    serde_saphyr::from_str::<HelperCase>(&source).expect("case should parse")
}

#[rstest]
fn replays_every_request_context_case_against_litellm(
    #[files("cases/helpers/mcp-request-context/*.yaml")] path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b8191");
    let run = run_case(&case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[rstest]
fn helper_seeds_select_b8191(
    #[files("fuzz/corpus/helper_case/mcp-request-context-*.yaml")] path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("seed should validate");
    assert_eq!(case.call.operation, "b8191");
}

#[test]
fn an_unknown_operation_is_not_a_b8191_input() {
    let input = json!({ "operation": "nope" });
    assert!(Op::check(&input).is_err());
}
