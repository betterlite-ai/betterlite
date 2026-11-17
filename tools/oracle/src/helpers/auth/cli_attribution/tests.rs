#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;

use super::Op;

#[rstest]
fn cli_attribution_cases_match_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/cli-attribution/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}

#[test]
fn a_literal_dot_destination_keeps_the_flat_claim() {
    let fixture =
        include_str!("../../../../cases/helpers/cli-attribution/literal-dot-before-traversal.yaml");
    let case: HelperCase = serde_saphyr::from_str(fixture).expect("helper case");
    let metadata = Op::decode_and_run(&case.call.input).expect("the build runs");
    assert_eq!(metadata["department"], "flat-value");
    assert_eq!(metadata["flat_department"], "flat-value");
    assert_eq!(metadata["team"], "nested-team");
}

#[test]
fn a_numeric_string_survives_the_merge() {
    let fixture =
        include_str!("../../../../cases/helpers/cli-attribution/numeric-string-preserved.yaml");
    let case: HelperCase = serde_saphyr::from_str(fixture).expect("helper case");
    let merged = Op::decode_and_run(&case.calls[0].input).expect("the merge runs");
    assert_eq!(merged["count"], "42");
    assert_eq!(merged["note"], "keep");
    assert_eq!(merged["profile"]["count"], "42");
    assert_eq!(merged["profile"]["tier"], 2);
}

#[test]
fn a_token_shaped_claim_never_reaches_the_metadata() {
    let fixture = include_str!(
        "../../../../cases/helpers/cli-attribution/token-shaped-and-secret-destinations.yaml"
    );
    let case: HelperCase = serde_saphyr::from_str(fixture).expect("helper case");
    let metadata = Op::decode_and_run(&case.call.input).expect("the build runs");
    assert!(metadata.get("jwt_note").is_none());
    assert!(metadata.get("access_token").is_none());
    assert_eq!(metadata["token_hint"], "eyJhbGciOiJIUzI1NiJ9");
    assert_eq!(metadata["display_name"], "shown");
}
