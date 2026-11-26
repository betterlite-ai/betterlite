#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

use super::Op;

#[rstest]
fn every_mcp_ceiling_case_agrees_with_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-ceilings/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).expect("case should be readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("case should parse");
    validate_helper(&path, &case).expect("case should validate");
    assert_eq!(case.call.operation, "b4670");
    let run = run_case(&[crate::GROUP], &case, false).expect("case should run");
    assert!(matches!(run, HelperRun::Match), "{}", path.display());
}

#[test]
fn an_unknown_field_is_an_invalid_input() {
    let input = serde_json::json!({
        "allowed": ["srv-a"],
        "agent_servers": [],
        "access_group_servers": null,
        "agent_group_servers": ["srv-a"],
    });
    let error = Op::decode_and_run(&input).expect_err("a field no ceiling reads is rejected");
    assert_eq!(error.class, "InvalidInput");
    assert!(
        error.message.contains("agent_group_servers"),
        "{}",
        error.message
    );
}
