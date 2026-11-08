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
fn replays_every_wire_results_case_against_litellm(
    #[files("cases/helpers/mcp-wire-results/*.yaml")] path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b8190");
    let run = run_case(&case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[rstest]
fn helper_seeds_select_b8190(
    #[files("fuzz/corpus/helper_case/mcp-wire-results-*.yaml")] path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("seed should validate");
    assert_eq!(case.call.operation, "b8190");
}

#[test]
fn an_unknown_function_is_not_a_b8190_input() {
    let input = json!({ "function": "nope" });
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_field_the_function_does_not_read_is_not_a_b8190_input() {
    let input = json!({ "function": "wire_compat_for", "protocol_version": "x", "extra": "y" });
    assert!(Op::check(&input).is_err());
}
