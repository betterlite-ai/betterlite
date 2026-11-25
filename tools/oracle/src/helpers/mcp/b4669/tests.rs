#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

use super::Op;

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

#[test]
fn a_key_permission_field_no_grant_reads_is_an_error() {
    let input = serde_json::json!({
        "function": "key",
        "servers": [],
        "access_groups": [],
        "toolset_grants": {},
        "permission": {"object_permission_id": "op-case", "mcp_serverz": ["srv-a"]},
        "access_group_ids": [],
    });
    let error = Op::decode_and_run(&input).expect_err("the misspelled grant field is rejected");
    assert_eq!(error.class, "InvalidInput");
    assert!(error.message.contains("mcp_serverz"), "{}", error.message);
}

#[test]
fn a_team_permission_field_no_grant_reads_is_an_error() {
    let input = serde_json::json!({
        "function": "team",
        "servers": [],
        "access_groups": [],
        "toolset_grants": {},
        "team_id": "team-1",
        "blocked": false,
        "team_access_group_ids": [],
        "permission": {"object_permission_id": "op-case", "skills": ["skill-1"]},
    });
    let error = Op::decode_and_run(&input).expect_err("the row field no grant reads is rejected");
    assert_eq!(error.class, "InvalidInput");
    assert!(error.message.contains("skills"), "{}", error.message);
}
