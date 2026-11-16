#![cfg(test)]

use opentelemetry::logs::AnyValue;
use rstest::rstest;

use crate::otel::legacy_metrics::events::tests::{
    Mode, attributes_of, expected_attributes, try_emit,
};
use crate::otel::legacy_policy::capture::CaptureMode;

const CALL: &str = r#"{
    "call_type": "acompletion",
    "litellm_params": {"custom_llm_provider": "openai"},
    "messages": [{"role": "user", "content": "hello"}]
}"#;
const REPLY: &str = r#"{"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}]}"#;
const INPUT_JSON: &str = r#"[{"parts":[{"content":"hello","type":"text"}],"role":"user"}]"#;
const OUTPUT_JSON: &str =
    r#"[{"finish_reason":"stop","parts":[{"content":"hi","type":"text"}],"role":"assistant"}]"#;
const EVENT: &str = "gen_ai.client.inference.operation.details";

#[test]
fn the_opt_in_emits_one_consolidated_event_with_both_message_lists() {
    let records =
        try_emit(Mode::Details, CaptureMode::SpanAndEvent, CALL, Some(REPLY)).expect("emit");
    assert_eq!(records.len(), 1);
    let record = records.first().expect("one record");
    assert_eq!(record.body(), None);
    assert_eq!(
        attributes_of(record),
        expected_attributes(&[
            ("event_name", AnyValue::from(EVENT)),
            ("gen_ai.provider.name", AnyValue::from("openai")),
            ("gen_ai.operation.name", AnyValue::from("chat")),
            ("gen_ai.input.messages", AnyValue::from(INPUT_JSON)),
            ("gen_ai.output.messages", AnyValue::from(OUTPUT_JSON)),
        ])
    );
}

#[rstest]
#[case::event_only(CaptureMode::EventOnly, true)]
#[case::span_and_event(CaptureMode::SpanAndEvent, true)]
#[case::span_only(CaptureMode::SpanOnly, false)]
#[case::no_content(CaptureMode::NoContent, false)]
fn the_messages_are_attached_only_when_the_event_captures_content(
    #[case] mode: CaptureMode,
    #[case] captured: bool,
) {
    let records = try_emit(Mode::Details, mode, CALL, Some(REPLY)).expect("emit");
    let attributes = attributes_of(records.first().expect("one record"));
    assert_eq!(attributes.contains_key("gen_ai.input.messages"), captured);
    assert_eq!(attributes.contains_key("gen_ai.output.messages"), captured);
    assert!(attributes.contains_key("gen_ai.operation.name"));
    assert!(attributes.contains_key("gen_ai.provider.name"));
}

#[rstest]
#[case::embedding("aembedding", "embeddings")]
#[case::text_completion("atext_completion", "text_completion")]
#[case::chat("acompletion", "chat")]
#[case::unknown("something_else", "chat")]
fn the_operation_name_comes_from_the_call_type(#[case] call_type: &str, #[case] expected: &str) {
    let kwargs = format!(r#"{{"call_type": "{call_type}"}}"#);
    let records = try_emit(Mode::Details, CaptureMode::NoContent, &kwargs, None).expect("emit");
    let attributes = attributes_of(records.first().expect("one record"));
    assert_eq!(
        attributes["gen_ai.operation.name"],
        AnyValue::from(expected.to_owned())
    );
}

#[test]
fn a_call_without_a_provider_has_no_provider_attribute() {
    let records = try_emit(Mode::Details, CaptureMode::NoContent, "{}", None).expect("emit");
    let attributes = attributes_of(records.first().expect("one record"));
    assert!(!attributes.contains_key("gen_ai.provider.name"));
    assert_eq!(attributes["gen_ai.operation.name"], AnyValue::from("chat"));
}

#[rstest]
#[case::no_messages(r#"{"call_type": "acompletion"}"#, Some(REPLY), false, true)]
#[case::null_messages(r#"{"messages": null}"#, Some(REPLY), false, true)]
#[case::empty_list(r#"{"messages": []}"#, Some(REPLY), false, true)]
#[case::empty_text(r#"{"messages": ""}"#, Some(REPLY), false, true)]
#[case::no_response(CALL, None, true, false)]
#[case::no_choices(CALL, Some(r#"{"id": "x"}"#), true, false)]
#[case::empty_choices(CALL, Some(r#"{"choices": []}"#), true, false)]
fn an_empty_message_list_is_not_an_attribute(
    #[case] kwargs: &str,
    #[case] response: Option<&str>,
    #[case] has_input: bool,
    #[case] has_output: bool,
) {
    let records = try_emit(Mode::Details, CaptureMode::EventOnly, kwargs, response).expect("emit");
    let attributes = attributes_of(records.first().expect("one record"));
    assert_eq!(attributes.contains_key("gen_ai.input.messages"), has_input);
    assert_eq!(
        attributes.contains_key("gen_ai.output.messages"),
        has_output
    );
}

#[test]
fn instructions_text_becomes_one_system_message() {
    let kwargs = r#"{"messages": "be brief"}"#;
    let records = try_emit(Mode::Details, CaptureMode::EventOnly, kwargs, None).expect("emit");
    let attributes = attributes_of(records.first().expect("one record"));
    assert_eq!(
        attributes["gen_ai.input.messages"],
        AnyValue::from(r#"[{"parts":[{"content":"be brief","type":"text"}],"role":"system"}]"#)
    );
}
