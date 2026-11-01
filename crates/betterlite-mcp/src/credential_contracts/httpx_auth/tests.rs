#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::{Auth as _, HeaderMap, NoOpAuth, StaticHeaderAuth};

#[rstest]
fn no_op_auth_attaches_nothing() {
    let mut headers = HeaderMap::new();
    NoOpAuth.apply(&mut headers);
    assert!(headers.is_empty());
}

#[rstest]
fn static_header_auth_defaults_to_authorization() {
    let mut headers = HeaderMap::new();
    StaticHeaderAuth {
        header_name: "Authorization".to_owned(),
        header_value: SecretString::from("bearer-token".to_owned()),
    }
    .apply(&mut headers);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("bearer-token")
    );
}

#[rstest]
fn static_header_auth_honors_a_custom_header_name() {
    let mut headers = HeaderMap::new();
    StaticHeaderAuth {
        header_name: "X-Api-Key".to_owned(),
        header_value: SecretString::from("api-key-value".to_owned()),
    }
    .apply(&mut headers);
    assert_eq!(
        headers.get("X-Api-Key").map(String::as_str),
        Some("api-key-value")
    );
    assert!(!headers.contains_key("Authorization"));
}
