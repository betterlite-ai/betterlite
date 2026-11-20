#![cfg(test)]

use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use rstest::rstest;

use crate::shadow_requests::Object;
use crate::shadow_requests::surfaces::Surface;

use super::post_guardrail_kwargs;

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn metadata(name: &str) -> Object {
    object(&format!(
        r#"{{"standard_logging_guardrail_information":[{{"guardrail_mode":"pre_call","guardrail_name":"{name}"}}]}}"#
    ))
}

fn captured(body: &str, request_metadata: &Object) -> GuardrailRequestSnapshot {
    GuardrailRequestSnapshot::capture(&object(body), request_metadata).unwrap()
}

#[test]
fn a_missing_snapshot_returns_none() {
    let current = metadata("g");
    assert_eq!(
        post_guardrail_kwargs(&object(r#"{"model":"m"}"#), &current, Surface::Chat, None),
        None
    );
}

#[test]
fn a_stale_fingerprint_returns_none() {
    let snapshot = captured(r#"{"messages":[]}"#, &metadata("first"));
    let kwargs = object(r#"{"model":"m"}"#);
    assert_eq!(
        post_guardrail_kwargs(&kwargs, &metadata("second"), Surface::Chat, Some(&snapshot)),
        None
    );
}

#[test]
fn clearing_the_guardrail_history_after_capture_makes_the_snapshot_stale() {
    let snapshot = captured(r#"{"messages":[]}"#, &metadata("g"));
    assert_eq!(
        post_guardrail_kwargs(&object("{}"), &object("{}"), Surface::Chat, Some(&snapshot)),
        None
    );
}

#[rstest]
#[case::chat(Surface::Chat)]
#[case::anthropic_messages(Surface::AnthropicMessages)]
fn message_surfaces_take_messages_from_the_body_messages_key(#[case] surface: Surface) {
    let current = metadata("g");
    let snapshot = captured(
        r#"{"messages":[{"role":"user","content":"new"}],"input":[{"role":"user","content":"raw"}]}"#,
        &current,
    );
    let kwargs = object(r#"{"model":"m","messages":[{"role":"user","content":"old"}]}"#);
    let rebuilt = post_guardrail_kwargs(&kwargs, &current, surface, Some(&snapshot)).unwrap();
    assert_eq!(
        rebuilt,
        object(
            r#"{"model":"m","messages":[{"role":"user","content":"new"}],"system":null,"instructions":null,
                "litellm_params":{"proxy_server_request":{"body":{"messages":[{"role":"user","content":"new"}],"input":[{"role":"user","content":"raw"}]}}}}"#
        )
    );
}

#[test]
fn the_responses_surface_takes_messages_from_the_body_input_key() {
    let current = metadata("g");
    let snapshot = captured(
        r#"{"input":[{"role":"user","content":"new"}],"messages":[{"role":"user","content":"old"}],"system":"s","instructions":"i"}"#,
        &current,
    );
    let kwargs = object(r#"{"model":"m","messages":"raw input"}"#);
    let rebuilt =
        post_guardrail_kwargs(&kwargs, &current, Surface::Responses, Some(&snapshot)).unwrap();
    assert_eq!(
        rebuilt,
        object(
            r#"{"model":"m","messages":[{"role":"user","content":"new"}],"system":"s","instructions":"i",
                "litellm_params":{"proxy_server_request":{"body":{"input":[{"role":"user","content":"new"}],"messages":[{"role":"user","content":"old"}],"system":"s","instructions":"i"}}}}"#
        )
    );
}

#[test]
fn the_snapshot_body_replaces_the_proxy_request_body_and_keeps_other_keys() {
    let current = metadata("g");
    let snapshot = captured(
        r#"{"messages":[{"role":"user","content":"new"}]}"#,
        &current,
    );
    let kwargs = object(
        r#"{"model":"m","litellm_params":{"api_key":"k","proxy_server_request":{"method":"POST","body":{"messages":[{"role":"user","content":"old"}]}}}}"#,
    );
    let rebuilt = post_guardrail_kwargs(&kwargs, &current, Surface::Chat, Some(&snapshot)).unwrap();
    assert_eq!(
        rebuilt,
        object(
            r#"{"model":"m","messages":[{"role":"user","content":"new"}],"system":null,"instructions":null,
                "litellm_params":{"api_key":"k","proxy_server_request":{"method":"POST","body":{"messages":[{"role":"user","content":"new"}]}}}}"#
        )
    );
}

#[test]
fn body_keys_that_are_absent_become_null() {
    let current = metadata("g");
    let snapshot = captured("{}", &current);
    let rebuilt = post_guardrail_kwargs(
        &object(r#"{"model":"m"}"#),
        &current,
        Surface::Responses,
        Some(&snapshot),
    )
    .unwrap();
    assert_eq!(
        rebuilt,
        object(
            r#"{"model":"m","messages":null,"system":null,"instructions":null,
                "litellm_params":{"proxy_server_request":{"body":{}}}}"#
        )
    );
}

#[test]
fn a_non_object_litellm_params_is_rebuilt_around_the_snapshot_body() {
    let current = metadata("g");
    let snapshot = captured("{}", &current);
    let rebuilt = post_guardrail_kwargs(
        &object(r#"{"model":"m","litellm_params":"x"}"#),
        &current,
        Surface::Chat,
        Some(&snapshot),
    )
    .unwrap();
    assert_eq!(
        rebuilt,
        object(
            r#"{"model":"m","messages":null,"system":null,"instructions":null,
                "litellm_params":{"proxy_server_request":{"body":{}}}}"#
        )
    );
}
