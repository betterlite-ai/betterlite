#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use uuid::Uuid;

use super::additional::{additional_model_fields, metrics_headers};
use super::environment_cases::{auth_and_metadata_headers, fake_stream_capability, malformed_json};
use super::support::{
    FIRST_ID, convert, fixture, instant, json, object, optional_params, rendered, supplied,
    with_tier,
};
use super::tools::{citation_content_alignment, tool_calls_response};
use crate::converse_response::ReplyFieldError;
use crate::converse_response::message::MessageError;
use crate::converse_response::transform::TransformError;
use crate::converse_response::wire::ReplyToolCall;

#[rstest]
#[case::converse_reply_envelope(converse_reply_envelope)]
#[case::stop_reasons(stop_reasons)]
#[case::metrics_headers(metrics_headers)]
#[case::additional_model_fields(additional_model_fields)]
#[case::fake_stream_capability(fake_stream_capability)]
#[case::malformed_json(malformed_json)]
#[case::tool_calls_response(tool_calls_response)]
#[case::citation_content_alignment(citation_content_alignment)]
#[case::auth_and_metadata_headers(auth_and_metadata_headers)]
fn converse_response_cases(#[case] check: fn()) {
    check();
}

const ENVELOPE: &str = r#"{"id":"chatcmpl-supplied","created":1700000000,"model":"fixture-model","object":"chat.completion","system_fingerprint":"fp-1","choices":[{"finish_reason":"stop","index":0,"message":{"content":"Hello!","role":"assistant","tool_calls":null,"function_call":null,"provider_specific_fields":null}}],"usage":{"completion_tokens":5,"prompt_tokens":10,"total_tokens":15,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":0,"rejected_prediction_tokens":null,"text_tokens":5,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":{"audio_tokens":null,"cache_write_tokens":0,"cached_tokens":0,"text_tokens":10,"image_tokens":null,"video_tokens":null,"cache_creation_tokens":0},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#;

const NULL_MESSAGE: &str = r#"{"id":"chatcmpl-supplied","created":0,"model":"fixture-model","object":"chat.completion","system_fingerprint":null,"choices":[{"finish_reason":"stop","index":0,"message":{"content":"","role":"assistant","tool_calls":null,"function_call":null,"provider_specific_fields":null}}],"usage":{"completion_tokens":1,"prompt_tokens":1,"total_tokens":2,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":0,"rejected_prediction_tokens":null,"text_tokens":1,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":{"audio_tokens":null,"cache_write_tokens":0,"cached_tokens":0,"text_tokens":1,"image_tokens":null,"video_tokens":null,"cache_creation_tokens":0},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#;

fn converse_reply_envelope() {
    envelope_rebuilds_the_supplied_response();
    envelope_treats_a_null_message_as_empty();
    envelope_keeps_supplied_tier_and_trace();
    a_clock_before_the_epoch_keeps_its_sign();
}

fn a_clock_before_the_epoch_keeps_its_sign() {
    let mut fixture = fixture();
    fixture.now = instant(-5);
    let body = r#"{"output": {"message": null}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.response().created, -5);
}

fn envelope_rebuilds_the_supplied_response() {
    let mut fixture = fixture();
    fixture.supplied = supplied("chatcmpl-supplied", Some("fp-1"));
    let body = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "Hello!"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture, body);
    assert_eq!(rendered(run.response()), json(ENVELOPE));
    assert_eq!(run.detail("input"), &fixture.messages);
    assert_eq!(
        run.detail("original_response"),
        &Json::Text(body.to_owned())
    );
    assert_eq!(
        run.detail("additional_args"),
        &json(r#"{"complete_input_dict": {"model": "fixture-model"}}"#)
    );
    assert_eq!(run.detail("log_event_type"), &json(r#""post_api_call""#));
    assert!(!run.details.contains_key("api_key"));
}

fn envelope_treats_a_null_message_as_empty() {
    let mut fixture = fixture();
    fixture.headers = Vec::new();
    fixture.now = instant(0);
    let body = r#"{"output": {"message": null}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture, body);
    assert_eq!(rendered(run.response()), json(NULL_MESSAGE));
}

fn envelope_keeps_supplied_tier_and_trace() {
    let mut fixture = fixture();
    fixture.model = String::from("requested-model");
    fixture.messages = json(r#"[{"role": "user", "content": "hello"}]"#);
    fixture.data = object(r#"{"model": "requested-model", "stream": true}"#);
    fixture.supplied = with_tier(
        supplied("chatcmpl-kept", None),
        r#""priority""#,
        r#"{"guardrail": "kept"}"#,
    );
    fixture.headers = vec![(String::from("x-amzn-requestid"), String::from("req-3"))];
    fixture.now = instant(1_700_000_001);
    let body = r#"{"output": {"message": {"role": "user", "content": [{"text": "Hi"}, {"text": ", there"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 2, "outputTokens": 3, "totalTokens": 5}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.response().id, "chatcmpl-kept");
    assert_eq!(run.response().model, "requested-model");
    assert_eq!(run.response().created, 1_700_000_001);
    assert_eq!(run.message().content.as_deref(), Some("Hi, there"));
    assert_eq!(run.message().role, "assistant");
    assert_eq!(run.response().service_tier, Some(json(r#""priority""#)));
    assert_eq!(run.response().trace, Some(json(r#"{"guardrail": "kept"}"#)));
    assert_eq!(run.detail("input"), &fixture.messages);
}

const STOP_REASONS: [(&str, &str); 10] = [
    ("end_turn", "stop"),
    ("stop_sequence", "stop"),
    ("max_tokens", "length"),
    ("tool_use", "tool_calls"),
    ("content_filtered", "content_filter"),
    ("guardrail_intervened", "content_filter"),
    ("malformed_model_output", "stop"),
    ("malformed_tool_use", "stop"),
    ("model_context_window_exceeded", "stop"),
    ("unexpected", "stop"),
];

fn stop_reasons() {
    mapped_stop_reasons();
    a_non_string_stop_reason_maps_to_stop_and_a_container_is_an_error();
    json_mode_forces_stop_when_every_tool_was_synthetic();
    json_mode_keeps_the_mapped_finish_without_synthetic_tools();
    textual_recovery_restores_tool_calls();
}

// Python's map lookup answers stop for a null, boolean or number stopReason, while a list or object is unhashable.
fn a_non_string_stop_reason_maps_to_stop_and_a_container_is_an_error() {
    for reason in ["null", "true", "42"] {
        let body = format!(
            r#"{{"output": {{"message": {{"content": []}}}}, "stopReason": {reason}, "usage": {{"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}}}"#
        );
        let run = convert(&fixture(), &body);
        assert_eq!(run.finish(), "stop", "stopReason {reason}");
    }
    for reason in ["[]", "{}"] {
        let body = format!(
            r#"{{"output": {{"message": {{"content": []}}}}, "stopReason": {reason}, "usage": {{"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}}}"#
        );
        let run = convert(&fixture(), &body);
        assert_eq!(run.status(), None, "stopReason {reason}");
        assert!(matches!(
            run.failure(),
            TransformError::Message(MessageError::Reply(ReplyFieldError::NotA {
                path: "stopReason",
                expected: "a string"
            }))
        ));
    }
}

fn json_mode_keeps_the_mapped_finish_without_synthetic_tools() {
    let mut fixture = fixture();
    fixture.optional =
        optional_params(r#"{"json_mode": true, "tools": [{"toolSpec": {"name": "real"}}]}"#);
    let body = r#"{"output": {"message": {"content": [{"text": "ok"}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert_eq!(run.message().content.as_deref(), Some("ok"));
}

fn mapped_stop_reasons() {
    for (reason, expected) in STOP_REASONS {
        let body = format!(
            r#"{{"output": {{"message": {{"content": [{{"text": "ok"}}]}}}}, "stopReason": "{reason}", "usage": {{"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}}}"#
        );
        let run = convert(&fixture(), &body);
        assert_eq!(run.finish(), expected, "stopReason {reason}");
    }
}

fn json_mode_forces_stop_when_every_tool_was_synthetic() {
    let mut fixture = fixture();
    fixture.optional =
        optional_params(r#"{"json_mode": true, "tools": [{"toolSpec": {"name": "real"}}]}"#);
    let body = r#"{"output": {"message": {"content": [{"toolUse": {"toolUseId": "tooluse_json", "name": "json_tool_call", "input": {"answer": 42}}}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "stop");
    assert_eq!(run.message().content.as_deref(), Some(r#"{"answer": 42}"#));
    assert!(run.message().tool_calls.is_none());
}

fn textual_recovery_restores_tool_calls() {
    let mut fixture = fixture();
    fixture.optional =
        optional_params(r#"{"json_mode": true, "tools": [{"toolSpec": {"name": "real"}}]}"#);
    fixture.ids = vec![Uuid::parse_str(FIRST_ID).expect("the fixture id parses")];
    let body = r#"{"output": {"message": {"content": [{"text": "{\"type\": \"function\", \"name\": \"real\", \"arguments\": \"{}\"}"}]}}, "stopReason": "tool_use", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;
    let run = convert(&fixture, body);
    assert_eq!(run.finish(), "tool_calls");
    assert!(run.message().content.is_none());
    let calls = run.message().tool_calls.as_deref().expect("a tool call");
    assert_eq!(calls.len(), 1);
    let ReplyToolCall::Recovered(call) = &calls[0] else {
        panic!("the recovered call keeps its own wire shape");
    };
    assert_eq!(call.id, FIRST_ID);
    assert_eq!(call.function.name.as_deref(), Some("real"));
    assert_eq!(call.function.arguments, "{}");
}
