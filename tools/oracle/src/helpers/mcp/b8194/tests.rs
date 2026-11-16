#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::json;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

use super::Op;

fn parse_case(path: &Path) -> HelperCase {
    let source = fs::read_to_string(path).expect("case should be readable");
    serde_saphyr::from_str::<HelperCase>(&source).expect("case should parse")
}

#[rstest]
fn replays_every_debug_case_against_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-debug/*.yaml")]
    path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b8194");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[rstest]
fn helper_seeds_select_b8194(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/mcp-debug-*.yaml")]
    path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("seed should validate");
    assert_eq!(case.call.operation, "b8194");
}

#[test]
fn an_unknown_operation_is_not_a_b8194_input() {
    let input = json!({ "operation": "nope" });
    assert!(Op::check(&input).is_err());
}

#[test]
fn an_unknown_top_level_key_is_not_a_b8194_input() {
    let input = json!({ "operation": "run", "bodys": [{ "body": "x" }] });
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_wrongly_typed_section_is_not_a_b8194_input() {
    let input = json!({ "operation": "run", "bodies": "nope" });
    assert!(Op::check(&input).is_err());
    let valid = json!({ "operation": "run", "bodies": [{ "body": "x" }] });
    Op::check(&valid).expect("a well-typed input should check");
}
