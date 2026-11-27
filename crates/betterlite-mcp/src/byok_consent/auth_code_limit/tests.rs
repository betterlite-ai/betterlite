#![cfg(test)]

use secrecy::SecretString;

use crate::byok_consent::ConsentError;
use crate::byok_consent::post::{PostForm, authorize_post};
use crate::byok_consent::store::{AUTH_CODES_MAX_SIZE, AuthCodeStore};
use crate::byok_consent::test_support::seeded_store;
use crate::byok_token_exchange::ports::AuthCodeLookup as _;

const NOW: f64 = 1_700_000_000.0;
const CALLBACK: &str = "http://127.0.0.1:1456/callback";

fn form() -> PostForm {
    PostForm {
        client_id: "client-1".to_owned(),
        redirect_uri: CALLBACK.to_owned(),
        code_challenge: "challenge".to_owned(),
        code_challenge_method: "S256".to_owned(),
        state: "state-1".to_owned(),
        server_id: "srv-1".to_owned(),
        api_key: SecretString::from("sk-user"),
    }
}

#[test]
fn the_cap_is_a_thousand_pending_flows() {
    assert_eq!(AUTH_CODES_MAX_SIZE, 1000);
}

#[test]
fn admit_allows_the_flow_that_brings_the_store_to_the_cap() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE - 1, NOW + 300.0);
    codes.admit().unwrap();
}

#[test]
fn nine_hundred_ninety_nine_pending_flows_admit_one_more() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE - 1, NOW + 300.0);
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    assert_eq!(codes.count(), AUTH_CODES_MAX_SIZE);
    assert!(codes.get(&accepted.code).is_some());
}

#[test]
fn a_thousand_pending_flows_refuse_the_next_one() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, NOW + 300.0);
    let error = authorize_post(&form(), "user-1", &codes, NOW).unwrap_err();
    assert_eq!(error.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::to_string(&error.body()).unwrap(),
        r#"{"detail":"Too many pending authorization flows"}"#
    );
    assert_eq!(codes.count(), AUTH_CODES_MAX_SIZE);
}

#[test]
fn admit_refuses_at_the_cap() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, NOW + 300.0);
    assert!(matches!(
        codes.admit(),
        Err(ConsentError::TooManyPendingFlows)
    ));
}

#[test]
fn expired_flows_do_not_count_toward_the_cap() {
    let codes = seeded_store(AUTH_CODES_MAX_SIZE, NOW - 1.0);
    let accepted = authorize_post(&form(), "user-1", &codes, NOW).unwrap();
    assert_eq!(codes.count(), 1);
    assert!(codes.get(&accepted.code).is_some());
}

#[test]
fn an_empty_store_admits_the_first_flow() {
    let codes = AuthCodeStore::new();
    codes.admit().unwrap();
    assert_eq!(codes.count(), 0);
}
