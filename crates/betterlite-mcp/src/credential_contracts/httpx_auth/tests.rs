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

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[rstest]
fn static_header_auth_defaults_to_authorization() {
    let mut headers = HeaderMap::new();
    StaticHeaderAuth::new("Authorization", &secret("bearer-token"))
        .expect("a plain header name and value are valid")
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
    StaticHeaderAuth::new("X-Api-Key", &secret("api-key-value"))
        .expect("a plain header name and value are valid")
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
    StaticHeaderAuth::new("Authorization", &secret("new-value"))
        .expect("a plain header name and value are valid")
        .apply(&mut headers);
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("new-value")
    );
}

#[rstest]
#[case::space_in_name("bad header")]
#[case::colon_in_name("bad:header")]
#[case::crlf_in_name("X-Api-Key\r\nInjected")]
fn static_header_auth_new_rejects_an_invalid_header_name(#[case] header_name: &str) {
    StaticHeaderAuth::new(header_name, &secret("value")).unwrap_err();
}

#[rstest]
#[case::crlf("value\r\nInjected: evil")]
#[case::null_byte("value\0evil")]
fn static_header_auth_new_rejects_an_invalid_header_value(#[case] header_value: &str) {
    StaticHeaderAuth::new("Authorization", &secret(header_value)).unwrap_err();
}

#[rstest]
fn static_header_auth_debug_redacts_the_header_value() {
    let auth = StaticHeaderAuth::new("Authorization", &secret("super-secret-token"))
        .expect("a plain header name and value are valid");
    assert!(!format!("{auth:?}").contains("super-secret-token"));
}
