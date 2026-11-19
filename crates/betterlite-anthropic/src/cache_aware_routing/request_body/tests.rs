#![cfg(test)]

use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use rstest::rstest;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::params;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }
    map
}

fn messages() -> Json {
    json(r#"[{"role":"user","content":"hi"}]"#)
}

fn body() -> Json {
    AnthropicCacheRouting::request_body(
        "https://api.anthropic.com/v1/messages",
        &headers(&[]),
        &params(r#"{"model":"claude-x","max_tokens":16}"#),
        &params(r#"{"max_tokens":32,"temperature":0.2}"#),
        Some(&messages()),
    )
    .expect("a native messages request")
}

#[test]
fn a_messages_url_with_a_query_is_accepted() {
    let merged = AnthropicCacheRouting::request_body(
        "https://api.anthropic.com/v1/messages?beta=true",
        &headers(&[]),
        &params(r#"{"model":"claude-x"}"#),
        &params("{}"),
        Some(&messages()),
    );
    assert!(merged.is_some());
}

#[rstest]
#[case::other_path("https://api.anthropic.com/v1/complete")]
#[case::short_path("https://api.anthropic.com/v1/message")]
#[case::lookalike("https://api.anthropic.com/xv1/messages")]
#[case::empty("")]
fn a_url_that_does_not_end_with_the_messages_path_is_rejected(#[case] url: &str) {
    assert_eq!(
        AnthropicCacheRouting::request_body(url, &headers(&[]), &params("{}"), &params("{}"), None),
        None
    );
}

#[test]
fn a_scheme_less_messages_url_is_accepted_like_urlparse() {
    let merged = AnthropicCacheRouting::request_body(
        "api.anthropic.com/v1/messages",
        &headers(&[]),
        &params("{}"),
        &params("{}"),
        None,
    );
    assert!(merged.is_some());
}

#[test]
fn the_anthropic_beta_header_rejects_the_request() {
    assert_eq!(
        AnthropicCacheRouting::request_body(
            "https://api.anthropic.com/v1/messages",
            &headers(&[("anthropic-beta", "prompt-caching-2024-07-31")]),
            &params("{}"),
            &params("{}"),
            None,
        ),
        None
    );
}

#[test]
fn a_foreign_anthropic_version_header_rejects_the_request() {
    assert_eq!(
        AnthropicCacheRouting::request_body(
            "https://api.anthropic.com/v1/messages",
            &headers(&[("anthropic-version", "2024-01-01")]),
            &params("{}"),
            &params("{}"),
            None,
        ),
        None
    );
}

#[test]
fn the_native_version_header_is_accepted() {
    assert!(
        AnthropicCacheRouting::request_body(
            "https://api.anthropic.com/v1/messages",
            &headers(&[("anthropic-version", "2023-06-01")]),
            &params("{}"),
            &params("{}"),
            None,
        )
        .is_some()
    );
}

#[test]
fn the_merged_body_carries_the_native_options_and_messages() {
    assert_eq!(
        body(),
        json(
            r#"{"model":"claude-x","max_tokens":32,"temperature":0.2,"messages":[{"role":"user","content":"hi"}]}"#
        )
    );
}
