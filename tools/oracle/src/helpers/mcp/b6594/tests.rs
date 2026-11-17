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
fn replays_every_server_utils_case_against_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-server-utils/*.yaml")]
    path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b6594");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[rstest]
fn helper_seeds_select_b6594(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/mcp-server-utils-*.yaml")]
    path: PathBuf,
) {
    let case = parse_case(&path);
    validate_helper(&path, &case).expect("seed should validate");
    assert_eq!(case.call.operation, "b6594");
}

#[test]
fn an_unknown_function_is_not_a_b6594_input() {
    let input = json!({ "function": "nope" });
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_field_the_function_does_not_read_is_not_a_b6594_input() {
    let input = json!({ "function": "setup_url", "server_id": "srv", "extra": "y" });
    assert!(Op::check(&input).is_err());
}
