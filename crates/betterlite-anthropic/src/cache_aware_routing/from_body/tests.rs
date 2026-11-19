#![cfg(test)]

use rstest::rstest;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::tests::support::BODY;
use crate::cache_aware_routing::tests::support::ONE_HOUR_BODY;
use crate::cache_aware_routing::tests::support::json;

#[test]
fn from_body_parses_the_prefix_and_the_output_limit() {
    let routing = AnthropicCacheRouting::from_body(&json(BODY)).expect("a predictable body");
    assert_eq!(routing.requested_output_limit, 64);
    assert_eq!(routing.prefix.ttl_seconds, 300);
    assert_eq!(routing.body, json(BODY));
}

#[test]
fn from_body_keeps_the_one_hour_ttl() {
    let routing =
        AnthropicCacheRouting::from_body(&json(ONE_HOUR_BODY)).expect("a predictable body");
    assert_eq!(routing.prefix.ttl_seconds, 3600);
}

#[rstest]
#[case::zero(r#"{"max_tokens":0,"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::negative(r#"{"max_tokens":-1,"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::float(r#"{"max_tokens":12.5,"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::text(r#"{"max_tokens":"12","messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::boolean(r#"{"max_tokens":true,"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::missing(r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]}"#)]
#[case::no_marker(r#"{"max_tokens":12,"messages":[{"role":"user","content":"hi"}]}"#)]
fn from_body_rejects_an_unusable_body(#[case] body: &str) {
    assert!(AnthropicCacheRouting::from_body(&json(body)).is_none());
}

#[test]
fn from_body_rejects_a_non_object_body() {
    assert!(AnthropicCacheRouting::from_body(&json("[]")).is_none());
}
