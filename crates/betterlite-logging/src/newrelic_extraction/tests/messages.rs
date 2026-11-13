use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::fixtures::{call_kwargs, content_policy, environment, json, object, payload, sources};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::timestamps::Moment;
use crate::newrelic_extraction::ExtractionError;
use crate::newrelic_extraction::content::{ContentPolicy, RECORD_CONTENT_ENV};
use crate::newrelic_extraction::messages::{MessageEvent, all_messages, message_content};

fn events_with(
    standard: Option<&StandardLoggingPayload>,
    kwargs_messages: Option<&Json>,
    response: &str,
    policy: &ContentPolicy<'_>,
) -> Result<Vec<MessageEvent>, ExtractionError> {
    let body = object(response);
    let mut kwargs = call_kwargs();
    kwargs.messages = kwargs_messages;
    let event_sources = sources(&kwargs, &body, standard);
    all_messages(&event_sources, "gpt-4o", "openai", policy)
}

fn zero_times(standard: &mut StandardLoggingPayload) {
    standard.start_time = Decimal::ZERO;
    standard.end_time = Decimal::ZERO;
}

#[rstest]
#[case::text(r#"{"content":"hello"}"#, "hello")]
#[case::missing("{}", "")]
#[case::null(r#"{"content":null}"#, "")]
#[case::number(r#"{"content":5}"#, "5")]
#[case::flag(r#"{"content":true}"#, "true")]
#[case::multimodal(
    r#"{"content":[{"type":"text","text":"hi"}]}"#,
    r#"[{"type": "text", "text": "hi"}]"#
)]
#[case::empty_list(r#"{"content":[]}"#, "[]")]
#[case::tool_calls(
    r#"{"content":"hi","tool_calls":[{"id":"call-1"}]}"#,
    r#"[{"id": "call-1"}]"#
)]
#[case::empty_tool_calls(r#"{"content":"hi","tool_calls":[]}"#, "hi")]
#[case::tool_calls_without_content(r#"{"tool_calls":[{"id":"call-1"}]}"#, r#"[{"id": "call-1"}]"#)]
fn newrelic_extraction_tests_behavior_message_content(
    #[case] encoded: &str,
    #[case] expected: &str,
) {
    let message = object(encoded);
    assert_eq!(message_content(&message), expected);
}

fn summarize(
    event: &MessageEvent,
) -> (
    usize,
    &str,
    Option<&str>,
    &str,
    &str,
    Option<bool>,
    Option<i64>,
) {
    (
        event.sequence,
        event.role.as_str(),
        event.content.as_deref(),
        event.response_model.as_str(),
        event.vendor.as_str(),
        event.is_response,
        event.timestamp,
    )
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_projects_request_and_response() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(
        r#"[{"role":"user","content":"hi"},{"role":"system","content":"rules"}]"#,
    ));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    let seen: Vec<_> = events.iter().map(summarize).collect();
    assert_eq!(
        seen,
        [
            (0, "user", Some("hi"), "gpt-4o", "openai", None, None),
            (1, "system", Some("rules"), "gpt-4o", "openai", None, None),
            (
                2,
                "assistant",
                Some("hello"),
                "gpt-4o",
                "openai",
                Some(true),
                None
            ),
        ]
    );
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_defaults_missing_roles() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"content":"hi"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].role, "user");
    assert_eq!(events[1].role, "assistant");
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_defaults_empty_roles() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"","content":"hi"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"role":"","content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].role, "user");
    assert_eq!(events[1].role, "assistant");
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_prefers_payload_messages() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"from payload"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let kwargs_messages = json(r#"[{"role":"user","content":"from kwargs"}]"#);
    let events = events_with(Some(&standard), Some(&kwargs_messages), "{}", &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content.as_deref(), Some("from payload"));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_falls_back_from_redacted_payload_messages() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#""redacted""#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let kwargs_messages = json(r#"[{"role":"user","content":"from kwargs"}]"#);
    let events = events_with(Some(&standard), Some(&kwargs_messages), "{}", &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content.as_deref(), Some("from kwargs"));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_keeps_an_explicit_empty_payload_list() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json("[]"));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let kwargs_messages = json(r#"[{"role":"user","content":"from kwargs"}]"#);
    let events = events_with(Some(&standard), Some(&kwargs_messages), "{}", &policy).unwrap();
    assert!(events.is_empty());
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_reads_kwargs_messages() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let kwargs_messages = json(r#"[{"role":"user","content":"from kwargs"}]"#);
    let events = events_with(Some(&standard), Some(&kwargs_messages), "{}", &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content.as_deref(), Some("from kwargs"));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_reads_a_delta_choice() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"delta":{"content":"streamed"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].role, "assistant");
    assert_eq!(events[0].content.as_deref(), Some("streamed"));
    assert_eq!(events[0].is_response, Some(true));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_prefers_a_message_choice() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"content":"message"},"delta":{"content":"delta"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content.as_deref(), Some("message"));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_falls_back_to_a_delta_after_a_null_message() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":null,"delta":{"content":"delta"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content.as_deref(), Some("delta"));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_skips_an_empty_choice_message() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let result = events_with(
        Some(&standard),
        None,
        r#"{"choices":[{"message":{}}]}"#,
        &policy,
    );
    assert!(result.unwrap().is_empty());
}

#[rstest]
#[case::leading_skip(r#"{"choices":[{"message":{}},{"message":{"content":"x"}}]}"#, 1)]
#[case::middle_skip(
    r#"{"choices":[{"message":{"content":"a"}},{"message":{}},{"message":{"content":"b"}}]}"#,
    2
)]
fn newrelic_extraction_tests_behavior_all_messages_numbers_skipped_choices_without_a_gap(
    #[case] response: &str,
    #[case] expected_sequence: usize,
) {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(
        events.last().map(|event| event.sequence),
        Some(expected_sequence)
    );
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_numbers_multiple_choices() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 0);
    assert_eq!(events[1].sequence, 1);
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_stamps_payload_times() {
    let mut standard = payload();
    standard.start_time = "1704067200.5".parse().unwrap();
    standard.end_time = "1704067201.25".parse().unwrap();
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events[0].timestamp, Some(1_704_067_200_500));
    assert_eq!(events[1].timestamp, Some(1_704_067_201_250));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_prefers_payload_times() {
    let mut standard = payload();
    standard.start_time = "1704067200.5".parse().unwrap();
    standard.end_time = "1704067201.25".parse().unwrap();
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[]);
    let policy = content_policy(&env);
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    let mut kwargs = call_kwargs();
    kwargs.messages = Some(&messages);
    kwargs.start_time = Some(Moment::Integer(1_000));
    kwargs.end_time = Some(Moment::Integer(2_000));
    let body = object(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let event_sources = sources(&kwargs, &body, Some(&standard));
    let events = all_messages(&event_sources, "gpt-4o", "openai", &policy).unwrap();
    assert_eq!(events[0].timestamp, Some(1_704_067_200_500));
    assert_eq!(events[1].timestamp, Some(1_704_067_201_250));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_stamps_kwargs_times() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    let mut kwargs = call_kwargs();
    kwargs.messages = Some(&messages);
    kwargs.start_time = Some(Moment::Integer(1_704_067_200));
    kwargs.end_time = Some(Moment::Integer(1_704_067_202));
    let body = object(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let event_sources = sources(&kwargs, &body, Some(&standard));
    let events = all_messages(&event_sources, "gpt-4o", "openai", &policy).unwrap();
    assert_eq!(events[0].timestamp, Some(1_704_067_200_000));
    assert_eq!(events[1].timestamp, Some(1_704_067_202_000));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_redaction_removes_content() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[]);
    let mut policy = content_policy(&env);
    policy.redact_message_logging = true;
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.content.is_none()));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_turn_off_removes_content() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[(RECORD_CONTENT_ENV, "false")]);
    let policy = content_policy(&env);
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.content.is_none()));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_turn_off_flag_removes_content() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[]);
    let mut policy = content_policy(&env);
    policy.turn_off_message_logging = true;
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.content.is_none()));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_both_switches_remove_content() {
    let mut standard = payload();
    zero_times(&mut standard);
    standard.messages = Some(json(r#"[{"role":"user","content":"hi"}]"#));
    let env = environment(&[(RECORD_CONTENT_ENV, "false")]);
    let mut policy = content_policy(&env);
    policy.redact_message_logging = true;
    let response = r#"{"choices":[{"message":{"content":"hello"}}]}"#;
    let events = events_with(Some(&standard), None, response, &policy).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.content.is_none()));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_rejects_a_non_object_request_message() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let kwargs_messages = json("[7]");
    let result = events_with(Some(&standard), Some(&kwargs_messages), "{}", &policy);
    assert!(matches!(result, Err(ExtractionError::RequestMessageShape)));
}

#[test]
fn newrelic_extraction_tests_behavior_all_messages_rejects_a_non_object_choice() {
    let mut standard = payload();
    zero_times(&mut standard);
    let env = environment(&[]);
    let policy = content_policy(&env);
    let non_object_choice = events_with(Some(&standard), None, r#"{"choices":[7]}"#, &policy);
    assert!(matches!(
        non_object_choice,
        Err(ExtractionError::ChoiceShape)
    ));

    let non_object_message = events_with(
        Some(&standard),
        None,
        r#"{"choices":[{"message":"text"}]}"#,
        &policy,
    );
    assert!(matches!(
        non_object_message,
        Err(ExtractionError::ChoiceShape)
    ));
}
