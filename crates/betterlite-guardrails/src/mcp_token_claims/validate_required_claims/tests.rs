#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use reqwest::StatusCode;
use rstest::rstest;

use crate::mcp_token_claims::validate_required_claims::validate;
use crate::mcp_token_claims::{ClaimError, Claims};

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

fn required(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_missing_claim_is_rejected_with_forbidden() {
    let error = validate(
        &required(&["sub", "email"]),
        Some(&claims(r#"{"sub":"alice"}"#)),
    )
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        error,
        ClaimError::MissingRequiredClaims {
            missing: vec!["email".to_owned()],
        }
    );
}

#[test]
fn the_rejection_lists_every_missing_claim_and_points_at_the_idp() {
    let error = validate(&required(&["sub", "email", "groups"]), None).unwrap_err();
    assert_eq!(
        error,
        ClaimError::MissingRequiredClaims {
            missing: vec!["sub".to_owned(), "email".to_owned(), "groups".to_owned(),],
        }
    );
    let message = error.to_string();
    assert!(message.contains("sub"));
    assert!(message.contains("email"));
    assert!(message.contains("groups"));
    assert!(message.contains("IdP"));
}

#[rstest]
#[case::null(Json::Null)]
#[case::false_value(Json::Bool(false))]
#[case::zero(Json::Number(serde_json::Number::from(0_i32)))]
#[case::empty_text(Json::Text(String::new()))]
#[case::empty_list(Json::List(Vec::new()))]
#[case::empty_object(Json::Object(Object::new()))]
fn a_claim_without_a_value_is_missing(#[case] value: Json) {
    let mut incoming = Claims::new();
    incoming.insert("sub".to_owned(), value);
    let error = validate(&required(&["sub"]), Some(&incoming)).unwrap_err();
    assert_eq!(
        error,
        ClaimError::MissingRequiredClaims {
            missing: vec!["sub".to_owned()],
        }
    );
}

#[rstest]
#[case::text(Json::Text("alice".to_owned()))]
#[case::true_value(Json::Bool(true))]
#[case::nonzero(Json::Number(serde_json::Number::from(7_i32)))]
#[case::nonempty_list(Json::List(vec![Json::Null]))]
#[case::nonempty_object(Json::Object(Object::from_iter([("k".to_owned(), Json::Null)])))]
fn a_claim_with_a_value_passes(#[case] value: Json) {
    let mut incoming = Claims::new();
    incoming.insert("sub".to_owned(), value);
    assert_eq!(validate(&required(&["sub"]), Some(&incoming)), Ok(()));
}

#[test]
fn an_empty_required_list_passes_without_any_claims() {
    assert_eq!(validate(&[], None), Ok(()));
    assert_eq!(validate(&required(&[]), Some(&Claims::new())), Ok(()));
}

#[test]
fn every_claim_is_missing_when_the_token_has_no_claims() {
    let error = validate(&required(&["sub", "email"]), None).unwrap_err();
    assert_eq!(
        error,
        ClaimError::MissingRequiredClaims {
            missing: vec!["sub".to_owned(), "email".to_owned()],
        }
    );
}

#[test]
fn claims_outside_the_required_list_are_not_checked() {
    let incoming = claims(r#"{"sub":"alice","groups":[]}"#);
    assert_eq!(validate(&required(&["sub"]), Some(&incoming)), Ok(()));
}
