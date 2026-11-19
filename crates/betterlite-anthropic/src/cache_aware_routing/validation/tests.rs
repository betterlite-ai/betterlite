#![cfg(test)]

use rstest::rstest;

use super::NATIVE_OPTIONS;
use super::native_body;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::params;

#[test]
fn native_body_keeps_every_body_field_and_applies_the_native_options() {
    let body = params(r#"{"model":"claude-x","max_tokens":32,"stream":false,"custom":true}"#);
    let request_kwargs = params(r#"{"max_tokens":64,"stream":true,"temperature":0.5,"rpm":10}"#);
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    let merged = native_body(&body, &request_kwargs, Some(&messages));
    assert_eq!(
        merged,
        json(
            r#"{"model":"claude-x","max_tokens":64,"stream":true,"custom":true,"temperature":0.5,"messages":[{"role":"user","content":"hi"}]}"#
        )
    );
}

#[test]
fn native_body_messages_argument_replaces_the_body_and_kwargs() {
    let body = params(r#"{"messages":[{"role":"user","content":"old"}]}"#);
    let request_kwargs = params(r#"{"messages":[{"role":"user","content":"kwargs"}]}"#);
    let messages = json(r#"[{"role":"user","content":"new"}]"#);
    let merged = native_body(&body, &request_kwargs, Some(&messages));
    assert_eq!(
        merged,
        json(r#"{"messages":[{"role":"user","content":"new"}]}"#)
    );
}

#[test]
fn native_body_writes_null_messages_when_the_caller_has_none() {
    let merged = native_body(&params("{}"), &params("{}"), None);
    assert_eq!(merged, json(r#"{"messages":null}"#));
}

#[rstest]
#[case::max_tokens("max_tokens")]
#[case::system("system")]
#[case::tools("tools")]
#[case::tool_choice("tool_choice")]
#[case::thinking("thinking")]
#[case::output_config("output_config")]
#[case::cache_control("cache_control")]
#[case::speed("speed")]
#[case::service_tier("service_tier")]
#[case::temperature("temperature")]
#[case::top_p("top_p")]
#[case::top_k("top_k")]
#[case::stop_sequences("stop_sequences")]
#[case::stream("stream")]
fn native_options_are_the_litellm_frozenset(#[case] option: &str) {
    assert!(NATIVE_OPTIONS.contains(&option));
    assert_eq!(NATIVE_OPTIONS.len(), 14);
}

#[rstest]
#[case::rpm("rpm")]
#[case::model("model")]
#[case::messages("messages")]
#[case::api_key("api_key")]
fn native_options_exclude_a_non_native_kwarg(#[case] option: &str) {
    assert!(!NATIVE_OPTIONS.contains(&option));
}
