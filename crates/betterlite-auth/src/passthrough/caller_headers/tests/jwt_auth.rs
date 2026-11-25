#![cfg(test)]

use rstest::rstest;

use super::caller_filter::is_authenticated_caller_jwt;
use super::support::{claims, jwt, secret};

fn authenticated(stored: &str, presented: &str) -> bool {
    is_authenticated_caller_jwt(&secret(&jwt(presented)), &claims(stored))
}

#[test]
fn the_claims_of_the_authenticated_token_match() {
    assert!(authenticated(
        r#"{"sub":"user-42"}"#,
        r#"{"sub":"user-42"}"#
    ));
}

#[test]
fn any_differing_stored_claim_refuses_the_value() {
    assert!(!authenticated(
        r#"{"sub":"user-42","scope":"a"}"#,
        r#"{"sub":"user-42","scope":"b"}"#
    ));
}

#[test]
fn a_presented_claim_the_store_does_not_carry_is_ignored() {
    assert!(authenticated(
        r#"{"sub":"user-42"}"#,
        r#"{"sub":"user-42","extra":1}"#
    ));
}

#[test]
fn an_absent_presented_claim_matches_a_stored_null() {
    assert!(authenticated(r#"{"sub":null}"#, "{}"));
}

#[test]
fn an_absent_presented_claim_does_not_match_a_stored_text() {
    assert!(!authenticated(r#"{"sub":"user-42"}"#, "{}"));
}

#[test]
fn nested_claims_compare_structurally() {
    assert!(authenticated(
        r#"{"a":{"b":[1,2]}}"#,
        r#"{"a":{"b":[1,2]}}"#
    ));
    assert!(!authenticated(
        r#"{"a":{"b":[1,2]}}"#,
        r#"{"a":{"b":[1,3]}}"#
    ));
}

// docs/deviations/B3603.yaml: JSON values compare by type, where Python's == equates 1, 1.0 and True
#[test]
fn claims_compare_by_their_json_value_and_type() {
    assert!(authenticated(r#"{"n":1}"#, r#"{"n":1}"#));
    assert!(!authenticated(r#"{"n":1}"#, r#"{"n":1.5}"#));
    assert!(!authenticated(r#"{"n":1}"#, r#"{"n":1.0}"#));
    assert!(!authenticated(r#"{"n":1}"#, r#"{"n":"1"}"#));
    assert!(!authenticated(r#"{"flag":true}"#, r#"{"flag":1}"#));
}

#[test]
fn a_unicode_claim_compares_by_text() {
    assert!(authenticated(r#"{"name":"é"}"#, r#"{"name":"é"}"#));
    assert!(!authenticated(r#"{"name":"é"}"#, r#"{"name":"e"}"#));
}

#[rstest]
#[case::issuer("_litellm_jwt_issuer")]
#[case::user_id("_litellm_user_id")]
#[case::user_email("_litellm_user_email")]
#[case::team_id("_litellm_team_id")]
#[case::team_ids("_litellm_team_ids")]
#[case::org_id("_litellm_org_id")]
#[case::end_user_id("_litellm_end_user_id")]
fn every_internal_claim_is_skipped(#[case] name: &str) {
    let stored = format!(r#"{{"{name}":"stored"}}"#);
    let presented = format!(r#"{{"{name}":"presented"}}"#);
    assert!(authenticated(&stored, &presented));
}

#[test]
fn a_public_claim_still_decides_after_internal_ones() {
    assert!(!authenticated(
        r#"{"sub":"a","_litellm_user_id":"x"}"#,
        r#"{"sub":"b","_litellm_user_id":"y"}"#
    ));
}

#[test]
fn an_empty_stored_claim_set_accepts_any_decodable_token() {
    assert!(authenticated("{}", r#"{"sub":"whoever"}"#));
}

#[test]
fn a_value_that_is_not_a_compact_jwt_never_matches() {
    assert!(!is_authenticated_caller_jwt(
        &secret("plain-secret"),
        &claims("{}")
    ));
    assert!(!is_authenticated_caller_jwt(
        &secret("two.parts"),
        &claims("{}")
    ));
    assert!(!is_authenticated_caller_jwt(
        &secret("not.a.jwt"),
        &claims("{}")
    ));
}
