#![cfg(test)]

use rstest::rstest;

use super::FakeStreamPort;
use crate::api::chat_agentic_loop::entry::maybe_run_chat_agentic_loop;
use crate::api::chat_agentic_loop::tests::support::{
    FakeContinuation, RecordingPort, callbacks, empty_stream, fixture, json_map, logging,
    optional_text, raw, response_text,
};
use crate::api::chat_agentic_loop::{ChatLoopResult, LoggingHandle, LoopPorts};

const MODEL_RESPONSE: &str =
    r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[]}"#;

#[test]
fn a_stream_stays_a_stream() {
    let port = RecordingPort::default();
    let logging = logging();
    let result = port.wrap(
        ChatLoopResult::Stream(empty_stream()),
        "gpt-4o",
        "openai",
        &LoggingHandle::Object(&logging),
    );
    assert!(matches!(result, ChatLoopResult::Stream(_)));
    assert!(port.records().is_empty());
}

#[test]
fn a_non_model_response_is_returned_unchanged() {
    let port = RecordingPort::default();
    let logging = logging();
    let result = port.wrap(
        ChatLoopResult::Response(raw(r#"{"id":"x"}"#)),
        "gpt-4o",
        "openai",
        &LoggingHandle::Object(&logging),
    );
    assert_eq!(response_text(result), r#"{"id":"x"}"#);
    assert!(port.records().is_empty());
}

#[test]
fn a_model_response_without_a_logging_object_is_unchanged() {
    let port = RecordingPort::default();
    let result = port.wrap(
        ChatLoopResult::Response(raw(MODEL_RESPONSE)),
        "gpt-4o",
        "openai",
        &LoggingHandle::Absent,
    );
    assert_eq!(response_text(result), MODEL_RESPONSE);
    assert!(port.records().is_empty());
}

#[test]
fn a_model_response_with_a_logging_object_is_wrapped() {
    let port = RecordingPort::default();
    let logging = logging();
    let result = port.wrap(
        ChatLoopResult::Response(raw(MODEL_RESPONSE)),
        "gpt-4o",
        "openai",
        &LoggingHandle::Object(&logging),
    );
    assert_eq!(response_text(result), r#"{"fake_stream":true}"#);
    let records = port.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].response, MODEL_RESPONSE);
    assert_eq!(records[0].model, "gpt-4o");
    assert_eq!(records[0].provider, "openai");
    assert!(records[0].has_logging);
}

#[rstest]
#[case::flag_at_zero("true", "0", true)]
#[case::flag_above_zero("true", "1", false)]
#[case::no_flag_at_zero("false", "0", false)]
#[case::number_flag_at_zero("1", "0", true)]
#[case::word_flag_at_zero("\"on\"", "0", true)]
#[tokio::test]
async fn the_final_wrap_needs_the_flag_and_depth_zero(
    #[case] flag: &'static str,
    #[case] depth: &'static str,
    #[case] wrapped: bool,
) {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[
        ("_code_interpreter_interception_converted_stream", flag),
        ("_agentic_loop_depth", depth),
    ]);
    let continuation = FakeContinuation::responding(ChatLoopResult::Response(raw("{}")));
    let port = RecordingPort::default();
    let ports = LoopPorts {
        continuation: &continuation,
        fake_stream: &port,
    };
    let handlers = callbacks(&[]);
    let result = maybe_run_chat_agentic_loop(&fixture.call(), &handlers, &ports)
        .await
        .unwrap();
    if wrapped {
        assert_eq!(optional_text(result), r#"{"fake_stream":true}"#);
        let records = port.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].response, MODEL_RESPONSE);
        assert_eq!(records[0].model, "gpt-4o");
        assert_eq!(records[0].provider, "openai");
    } else {
        assert!(result.is_none());
        assert!(port.records().is_empty());
    }
}

#[tokio::test]
async fn the_headroom_flag_also_wraps_without_callbacks() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("_headroom_interception_converted_stream", "true")]);
    let continuation = FakeContinuation::responding(ChatLoopResult::Response(raw("{}")));
    let port = RecordingPort::default();
    let ports = LoopPorts {
        continuation: &continuation,
        fake_stream: &port,
    };
    let result = maybe_run_chat_agentic_loop(&fixture.call(), &callbacks(&[]), &ports)
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"fake_stream":true}"#);
    assert_eq!(port.records()[0].response, MODEL_RESPONSE);
}
