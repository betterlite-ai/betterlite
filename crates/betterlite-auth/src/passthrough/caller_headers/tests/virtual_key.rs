#![cfg(test)]

use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::credential_value::get_litellm_virtual_key;
use super::support::headers;

#[rstest]
#[case::litellm_key_header_only(&[("x-litellm-api-key", "sk-virtual")], "Bearer sk-virtual")]
#[case::litellm_key_before_authorization(
    &[("Authorization", "Bearer sk-caller"), ("x-litellm-api-key", "sk-virtual")],
    "Bearer sk-virtual"
)]
#[case::litellm_key_is_not_normalized(
    &[("x-litellm-api-key", "Bearer sk-virtual")],
    "Bearer Bearer sk-virtual"
)]
#[case::authorization_only(&[("Authorization", "Bearer sk-caller")], "Bearer sk-caller")]
#[case::bare_authorization(&[("authorization", "sk-caller")], "sk-caller")]
#[case::basic_authorization(&[("Authorization", "Basic dXNlcjpwYXNz")], "Basic dXNlcjpwYXNz")]
#[case::upper_case_names(
    &[("AUTHORIZATION", "sk-caller"), ("X-LITELLM-API-KEY", "sk-virtual")],
    "Bearer sk-virtual"
)]
#[case::empty_authorization(&[("Authorization", "")], "")]
#[case::no_headers(&[], "")]
fn the_litellm_key_header_wins_and_authorization_is_the_fallback(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let key = get_litellm_virtual_key(&headers(pairs));
    assert!(key.expose_secret() == expected);
}

// docs/compatibility.md: an explicit empty value is used as given, where Python truthiness would fall back
#[test]
fn an_empty_litellm_key_header_still_gets_the_bearer_prefix() {
    let key = get_litellm_virtual_key(&headers(&[("x-litellm-api-key", "")]));
    assert!(key.expose_secret() == "Bearer ");
}

#[test]
fn a_repeated_litellm_key_header_keeps_its_first_value() {
    let mut map = HeaderMap::new();
    map.append("x-litellm-api-key", HeaderValue::from_static("first"));
    map.append("x-litellm-api-key", HeaderValue::from_static("second"));
    let key = get_litellm_virtual_key(&map);
    assert!(key.expose_secret() == "Bearer first");
}

#[test]
fn an_authorization_value_decodes_from_latin_1_bytes() {
    let mut map = HeaderMap::new();
    let value = HeaderValue::from_bytes(b"\xe9").expect("obs-text stays a header value");
    map.insert(AUTHORIZATION, value);
    let key = get_litellm_virtual_key(&map);
    assert!(key.expose_secret() == "\u{e9}");
}
