#![cfg(test)]

use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use rstest::rstest;

use super::litellm_key_from_request;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

#[rstest]
#[case::api_key_header_wins(&[("x-litellm-api-key", "key-1"), ("authorization", "Bearer key-2")], Some("key-1"))]
#[case::authorization_bearer(&[("authorization", "Bearer token-1")], Some("token-1"))]
#[case::lowercase_bearer(&[("authorization", "bearer token-2")], Some("token-2"))]
#[case::mixed_case_bearer(&[("authorization", "bEaReR token-3")], Some("token-3"))]
#[case::bare_token(&[("authorization", "token-4")], Some("token-4"))]
#[case::surrounding_space(&[("authorization", "  token-5 \t")], Some("token-5"))]
#[case::python_space(&[("authorization", "\u{1c}token-6\u{1f}")], Some("token-6"))]
#[case::space_after_bearer(&[("authorization", "Bearer    token-7")], Some("token-7"))]
#[case::empty_api_key_falls_through(&[("x-litellm-api-key", ""), ("authorization", "token-8")], Some("token-8"))]
#[case::blank_api_key_falls_through(&[("x-litellm-api-key", " \u{1e}"), ("authorization", "Bearer token-9")], Some("token-9"))]
#[case::bare_bearer_word(&[("authorization", "Bearer ")], Some("Bearer"))]
#[case::bearer_without_token(&[("authorization", "bearer    ")], Some("bearer"))]
#[case::no_headers(&[], None)]
#[case::empty_authorization(&[("authorization", "")], None)]
#[case::blank_authorization(&[("authorization", " \t ")], None)]
#[case::empty_api_key_only(&[("x-litellm-api-key", "")], None)]
fn litellm_key_from_request_reads_the_headers_in_litellm_order(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        litellm_key_from_request(&headers(pairs)).as_deref(),
        expected
    );
}

#[test]
fn litellm_key_from_request_decodes_header_bytes_as_latin1() {
    let mut map = HeaderMap::new();
    map.insert(
        "x-litellm-api-key",
        HeaderValue::from_bytes(&[0xE9, b'x']).unwrap(),
    );
    assert_eq!(litellm_key_from_request(&map), Some("\u{e9}x".to_owned()));
}

#[test]
fn litellm_key_from_request_takes_only_the_first_repeated_header() {
    let map = headers(&[
        ("authorization", "Bearer first"),
        ("authorization", "second"),
    ]);
    assert_eq!(litellm_key_from_request(&map), Some("first".to_owned()));
}
