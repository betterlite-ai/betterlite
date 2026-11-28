#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::Op;

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).unwrap()
}

#[test]
fn a_presented_key_header_beats_authorization() {
    let input = json!({
        "operation": "presented_key",
        "headers": [
            ["x-litellm-api-key", "sk-header"],
            ["authorization", "Bearer sk-authorization"],
        ],
    });
    assert_eq!(run(&input), json!("sk-header"));
}

#[rstest]
#[case::lower("bearer sk-bear")]
#[case::upper("BEARER sk-bear")]
#[case::mixed("bEaReR sk-bear")]
fn a_bearer_prefix_is_dropped_in_any_case(#[case] header: &str) {
    let input = json!({"operation": "presented_key", "headers": [["authorization", header]]});
    assert_eq!(run(&input), json!("sk-bear"));
}

#[test]
fn an_empty_x_litellm_api_key_falls_through() {
    let input = json!({
        "operation": "presented_key",
        "headers": [
            ["x-litellm-api-key", ""],
            ["authorization", "sk-authorization"],
        ],
    });
    assert_eq!(run(&input), json!("sk-authorization"));
}

#[test]
fn a_bare_bearer_word_is_the_token() {
    let input = json!({"operation": "presented_key", "headers": [["authorization", "Bearer "]]});
    assert_eq!(run(&input), json!("Bearer"));
}

#[test]
fn header_bytes_are_decoded_as_latin1() {
    let input = json!({"operation": "presented_key", "headers": [["authorization", "sk-café"]]});
    assert_eq!(run(&input), json!("sk-café"));
}

#[test]
fn no_presented_token_is_null() {
    let input = json!({"operation": "presented_key", "headers": []});
    assert_eq!(run(&input), json!(null));
}

#[rstest]
#[case::blocked(json!({"operation": "key_is_active", "key": {"token": "sk-blocked", "blocked": true}}), false)]
#[case::future(
    json!({"operation": "key_is_active", "key": {"token": "sk-future", "user_id": "user-1", "expires": "2099-01-01T00:00:00+05:30"}}),
    true
)]
#[case::past(
    json!({"operation": "key_is_active", "key": {"token": "sk-past", "expires": "2000-01-01T00:00:00Z"}}),
    false
)]
#[case::bad_expiry(
    json!({"operation": "key_is_active", "key": {"token": "sk-text", "expires": "not-a-date"}}),
    false
)]
fn the_active_gate_matches_the_expiry(#[case] input: Value, #[case] expected: bool) {
    assert_eq!(run(&input), json!(expected));
}

#[rstest]
#[case::active(
    json!({"operation": "active_key_user_id", "key": {"token": "sk-live", "user_id": "user-1"}}),
    json!("user-1")
)]
#[case::team_key(
    json!({"operation": "active_key_user_id", "key": {"token": "sk-team", "team_id": "team-a"}}),
    json!(null)
)]
#[case::expired(
    json!({"operation": "active_key_user_id", "key": {"token": "sk-old", "user_id": "user-1", "expires": "2000-01-01T00:00:00Z"}}),
    json!(null)
)]
fn the_active_user_id_comes_from_a_live_key(#[case] input: Value, #[case] expected: Value) {
    assert_eq!(run(&input), expected);
}

#[rstest]
#[case::scim_false(json!({"user_id": "user-1", "metadata": {"scim_active": false}}), "no_active_key")]
#[case::scim_zero(json!({"user_id": "user-1", "metadata": {"scim_active": 0}}), "active")]
#[case::metadata_null(json!({"user_id": "user-1", "metadata": null}), "active")]
#[case::no_metadata(json!({"user_id": "user-1"}), "active")]
fn the_user_record_is_active_unless_scim_says_false(#[case] user: Value, #[case] expected: &str) {
    let input = json!({"operation": "user_record", "user": user});
    assert_eq!(run(&input), json!(expected));
}

#[rstest]
fn replays_the_mcp_active_subject_cases(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-active-subjects/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}
