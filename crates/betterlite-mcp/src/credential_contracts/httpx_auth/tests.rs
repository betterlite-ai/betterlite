#![cfg(test)]

use http::{HeaderName, HeaderValue};
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
        headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok()),
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
        headers
            .get("X-Api-Key")
            .and_then(|value| value.to_str().ok()),
        Some("api-key-value")
    );
    assert!(!headers.contains_key("Authorization"));
}

#[rstest]
fn static_header_auth_overwrites_an_existing_header_regardless_of_case() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("old-value"),
    );
    StaticHeaderAuth {
        header_name: "Authorization".to_owned(),
        header_value: SecretString::from("new-value".to_owned()),
    }
    .apply(&mut headers);
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("new-value")
    );
}
