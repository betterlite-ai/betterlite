#![cfg(test)]

use opentelemetry::logs::AnyValue;
use rstest::rstest;

use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::legacy_metrics::events::tests::{
    Mode, attributes_of, expected_attributes, try_emit,
};
use crate::otel::legacy_policy::capture::CaptureMode;

const PROMPT: &str = "gen_ai.content.prompt";
const COMPLETION: &str = "gen_ai.content.completion";

fn map(pairs: Vec<(&'static str, AnyValue)>) -> AnyValue {
    AnyValue::from_iter(pairs)
}

fn call(messages: &str) -> String {
    format!(r#"{{"litellm_params": {{"custom_llm_provider": "openai"}}, "messages": {messages}}}"#)
}

fn reply(choices: &str) -> String {
    format!(r#"{{"choices": {choices}}}"#)
}

#[test]
fn each_message_becomes_a_prompt_event_with_its_content() {
    let kwargs = call(
        r#"[{"role": "system", "content": "be brief"}, {"role": "user", "content": "hello"}]"#,
    );
    let records = try_emit(Mode::Legacy, CaptureMode::SpanAndEvent, &kwargs, None).expect("emit");
    assert_eq!(records.len(), 2);
    let second = records.get(1).expect("second record");
    assert_eq!(
        attributes_of(second),
        expected_attributes(&[
            ("event_name", AnyValue::from(PROMPT)),
            ("gen_ai.system", AnyValue::from("openai")),
            ("gen_ai.prompt", AnyValue::from("hello")),
        ])
    );
    assert_eq!(
        second.body(),
        Some(&map(vec![
            ("role", AnyValue::from("user")),
            ("content", AnyValue::from("hello")),
        ]))
    );
    let first = records.first().expect("first record");
    assert_eq!(
        attributes_of(first)["gen_ai.prompt"],
        AnyValue::from("be brief")
    );
}

#[test]
fn each_choice_becomes_a_completion_event_with_index_and_finish_reason() {
    let choices = r#"[
        {"finish_reason": "length", "message": {"role": "assistant", "content": "one"}},
        {"finish_reason": "stop", "message": {"role": "assistant", "content": "two"}}
    ]"#;
    let records = try_emit(
        Mode::Legacy,
        CaptureMode::EventOnly,
        "{}",
        Some(&reply(choices)),
    )
    .expect("emit");
    assert_eq!(records.len(), 2);
    let second = records.get(1).expect("second record");
    assert_eq!(
        attributes_of(second),
        expected_attributes(&[
            ("event_name", AnyValue::from(COMPLETION)),
            ("index", AnyValue::from(1_i64)),
            ("finish_reason", AnyValue::from("stop")),
            ("message.content", AnyValue::from("two")),
        ])
    );
    assert_eq!(
        second.body(),
        Some(&map(vec![
            ("index", AnyValue::from(1_i64)),
            ("finish_reason", AnyValue::from("stop")),
            (
                "message",
                map(vec![
                    ("role", AnyValue::from("assistant")),
                    ("content", AnyValue::from("two")),
                ]),
            ),
        ]))
    );
    let first = records.first().expect("first record");
    assert_eq!(attributes_of(first)["index"], AnyValue::from(0_i64));
}

#[test]
fn prompts_come_before_completions() {
    let kwargs = call(r#"[{"role": "user", "content": "hello"}]"#);
    let choices = r#"[{"message": {"role": "assistant", "content": "hi"}}]"#;
    let records = try_emit(
        Mode::Legacy,
        CaptureMode::SpanAndEvent,
        &kwargs,
        Some(&reply(choices)),
    )
    .expect("emit");
    let names: Vec<AnyValue> = records
        .iter()
        .map(|record| attributes_of(record)["event_name"].clone())
        .collect();
    assert_eq!(names, [AnyValue::from(PROMPT), AnyValue::from(COMPLETION)]);
}

#[rstest]
#[case::event_only(CaptureMode::EventOnly, true)]
#[case::span_and_event(CaptureMode::SpanAndEvent, true)]
#[case::span_only(CaptureMode::SpanOnly, false)]
#[case::no_content(CaptureMode::NoContent, false)]
fn content_is_attached_only_when_the_event_captures_it(
    #[case] mode: CaptureMode,
    #[case] captured: bool,
) {
    let kwargs = call(r#"[{"role": "user", "content": "hello"}]"#);
    let choices =
        r#"[{"finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}]"#;
    let records = try_emit(Mode::Legacy, mode, &kwargs, Some(&reply(choices))).expect("emit");
    let prompt = records.first().expect("prompt");
    let completion = records.get(1).expect("completion");
    assert_eq!(
        attributes_of(prompt).contains_key("gen_ai.prompt"),
        captured
    );
    assert_eq!(
        attributes_of(completion).contains_key("message.content"),
        captured
    );
    let prompt_body = if captured {
        map(vec![
            ("role", AnyValue::from("user")),
            ("content", AnyValue::from("hello")),
        ])
    } else {
        map(vec![("role", AnyValue::from("user"))])
    };
    assert_eq!(prompt.body(), Some(&prompt_body));
    let message = if captured {
        map(vec![
            ("role", AnyValue::from("assistant")),
            ("content", AnyValue::from("hi")),
        ])
    } else {
        map(vec![("role", AnyValue::from("assistant"))])
    };
    let completion_body = map(vec![
        ("index", AnyValue::from(0_i64)),
        ("finish_reason", AnyValue::from("stop")),
        ("message", message),
    ]);
    assert_eq!(completion.body(), Some(&completion_body));
}

#[rstest]
#[case::tool_with_id(r#"{"role": "tool", "id": "call-1", "content": "x"}"#, Some("call-1"))]
#[case::tool_without_id(r#"{"role": "tool", "content": "x"}"#, None)]
#[case::tool_with_empty_id(r#"{"role": "tool", "id": "", "content": "x"}"#, None)]
#[case::user_with_id(r#"{"role": "user", "id": "call-1", "content": "x"}"#, None)]
#[case::missing_role_is_a_user(r#"{"id": "call-1", "content": "x"}"#, None)]
fn only_a_tool_message_carries_its_id_as_an_attribute(
    #[case] message: &str,
    #[case] expected: Option<&str>,
) {
    let kwargs = call(&format!("[{message}]"));
    let records = try_emit(Mode::Legacy, CaptureMode::EventOnly, &kwargs, None).expect("emit");
    let attributes = attributes_of(records.first().expect("prompt"));
    let found = attributes.get("id").cloned();
    assert_eq!(found, expected.map(|id| AnyValue::from(id.to_owned())));
}

#[rstest]
#[case::empty_text(r#"{"role": "user", "content": ""}"#)]
#[case::null(r#"{"role": "user", "content": null}"#)]
#[case::absent(r#"{"role": "user"}"#)]
#[case::list_of_parts(r#"{"role": "user", "content": [{"type": "text", "text": "hi"}]}"#)]
fn only_non_empty_text_content_is_a_prompt_attribute(#[case] message: &str) {
    let kwargs = call(&format!("[{message}]"));
    let records = try_emit(Mode::Legacy, CaptureMode::EventOnly, &kwargs, None).expect("emit");
    let attributes = attributes_of(records.first().expect("prompt"));
    assert!(!attributes.contains_key("gen_ai.prompt"));
}

#[test]
fn a_list_of_parts_stays_in_the_body() {
    let kwargs = call(r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}]}]"#);
    let records = try_emit(Mode::Legacy, CaptureMode::EventOnly, &kwargs, None).expect("emit");
    let part = map(vec![
        ("type", AnyValue::from("text")),
        ("text", AnyValue::from("hi")),
    ]);
    let expected = map(vec![
        ("role", AnyValue::from("user")),
        ("content", AnyValue::from_iter([part])),
    ]);
    assert_eq!(records.first().expect("prompt").body(), Some(&expected));
}

#[test]
fn a_choice_without_a_message_or_finish_reason_still_names_the_assistant() {
    let choices = r#"[{"finish_reason": null}, {"message": {"content": "hi"}}]"#;
    let records = try_emit(
        Mode::Legacy,
        CaptureMode::EventOnly,
        "{}",
        Some(&reply(choices)),
    )
    .expect("emit");
    let bare = records.first().expect("first");
    assert!(!attributes_of(bare).contains_key("finish_reason"));
    let role_only = map(vec![("role", AnyValue::from("assistant"))]);
    assert_eq!(
        bare.body(),
        Some(&map(vec![
            ("index", AnyValue::from(0_i64)),
            ("message", role_only)
        ]))
    );
    let without_role = records.get(1).expect("second");
    let message = map(vec![
        ("role", AnyValue::from("assistant")),
        ("content", AnyValue::from("hi")),
    ]);
    assert_eq!(
        without_role.body(),
        Some(&map(vec![
            ("index", AnyValue::from(1_i64)),
            ("message", message)
        ]))
    );
}

#[test]
fn a_call_without_a_provider_has_no_system_attribute() {
    let records = try_emit(
        Mode::Legacy,
        CaptureMode::EventOnly,
        r#"{"messages": [{"role": "user", "content": "hello"}]}"#,
        None,
    )
    .expect("emit");
    assert!(!attributes_of(records.first().expect("prompt")).contains_key("gen_ai.system"));
}

#[rstest]
#[case::text_message(r#"{"messages": ["text"]}"#, None, "messages")]
#[case::text_choice("{}", Some(r#"{"choices": ["text"]}"#), "choices")]
fn an_item_that_is_not_an_object_is_an_error_naming_the_field(
    #[case] kwargs: &str,
    #[case] response: Option<&str>,
    #[case] field: &str,
) {
    let error = try_emit(Mode::Legacy, CaptureMode::EventOnly, kwargs, response)
        .expect_err("not an object");
    assert!(
        matches!(&error, LegacyMetricsError::WrongType { field: named, .. } if *named == field)
    );
}
