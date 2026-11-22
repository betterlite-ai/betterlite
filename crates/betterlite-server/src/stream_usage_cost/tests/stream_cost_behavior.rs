use std::borrow::Cow;
use std::str::from_utf8;

use betterlite_anthropic::usage::ReasoningTokenCounter as _;
use rstest::rstest;

use super::super::MAX_INSPECTED_BYTES;
use super::super::StreamCostInjector;
use super::super::decode_cost_frame;
use super::super::projection::UnusedReasoningCounter;
use super::super::testing::{FixedCalculator, FixedRecorder};
use super::{
    CHAT_EVENT, DELTA_EVENT, Json, RESPONSE_COST_DEBUG, amount, breakdown, calculator, call_log,
    cost_text, details_object, injector, json, recorder, take_calls,
};

const EMPTY_FRAME_LEN: usize =
    "data: {\"object\":\"chat.completion.chunk\",\"usage\":{},\"pad\":\"\"}\n\n".len();

#[rstest]
#[case::bytes_with_lf_lf("\n\n", true)]
#[case::bytes_with_crlf_crlf("\r\n\r\n", true)]
#[case::bytes_with_one_lf("\n", false)]
#[case::bytes_without_a_terminator("", false)]
fn byte_frames_need_a_complete_terminator(#[case] terminator: &str, #[case] rewritten: bool) {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let frame = format!("data: {CHAT_EVENT}{terminator}");
    let output = injector.process_bytes(frame.as_bytes(), "gpt-4o", None);
    if rewritten {
        let text = from_utf8(&output).unwrap();
        assert!(text.ends_with(terminator), "the separator is preserved");
        assert!(text.contains("\"cost\":0.5"));
        assert!(take_calls(&calculator.observed).is_empty());
    } else {
        assert!(matches!(output, Cow::Borrowed(_)));
        assert_eq!(output.as_ref(), frame.as_bytes());
        assert!(take_calls(&calculator.observed).is_empty());
    }
}

#[test]
fn a_text_frame_is_rewritten_without_a_terminator() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.25"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {CHAT_EVENT}");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(output.ends_with("\n\n"));
    assert!(output.contains("\"cost\":0.25"));
}

#[test]
fn a_crlf_text_result_gets_the_extra_lf_lf() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.25"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {CHAT_EVENT}\r\n\r\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(output.ends_with("\r\n\r\n\n\n"));
}

#[test]
fn an_already_terminated_text_result_is_not_padded() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.25"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert_eq!(output.matches("\n\n").count(), 1);
    assert!(output.ends_with("\n\n"));
}

#[test]
fn the_first_eligible_data_line_wins() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {CHAT_EVENT}\ndata: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    let mut lines = output.split('\n');
    let first = lines.next().unwrap();
    let second = lines.next().unwrap();
    assert!(first.contains("\"cost\":0.5"));
    assert_eq!(second, format!("data: {CHAT_EVENT}"));
    assert_eq!(take_calls(&calculator.observed).len(), 1);
}

#[test]
fn an_event_without_usage_lets_the_next_line_be_priced() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {{\"object\":\"chat.completion.chunk\"}}\ndata: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(output.contains("\"cost\":0.5"));
    assert_eq!(take_calls(&calculator.observed).len(), 1);
}

#[test]
fn a_done_marker_is_skipped() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: [DONE]\ndata: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(output.starts_with("data: [DONE]\n"));
    assert!(output.contains("\"cost\":0.5"));
}

#[test]
fn a_malformed_data_line_aborts_the_attempt() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: {{not json\ndata: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(matches!(output, Cow::Borrowed(_)));
    assert_eq!(output.as_ref(), source.as_str());
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_non_object_data_value_aborts_the_attempt() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("data: [1,2]\ndata: {CHAT_EVENT}\n\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    assert_eq!(output.as_ref(), source.as_str());
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn crlf_is_preserved_on_the_rewritten_line() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = format!("event: message\r\ndata: {CHAT_EVENT}\r\n\r\n");
    let output = injector.process_text(&source, "gpt-4o", None);
    let rewritten = output.split('\n').nth(1).unwrap();
    assert!(rewritten.ends_with('\r'));
    assert!(rewritten.contains("\"cost\":0.5"));
    assert!(output.starts_with("event: message\r\n"));
}

#[test]
fn unrelated_fields_and_lines_are_preserved() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(
        r#"{"id":"chunk-1","object":"chat.completion.chunk","choices":[{"index":0}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#,
    );
    let output = injector.process_object(&event, "gpt-4o", None);
    let Json::Object(fields) = output.as_ref() else {
        panic!("the object stays an object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text("chunk-1".to_owned())));
    assert_eq!(fields.get("choices"), event_field(&event, "choices"));
    assert_eq!(fields.get("object"), event_field(&event, "object"));
    assert_eq!(cost_text(output.as_ref()), Some("0.5".to_owned()));
    assert_eq!(take_calls(&calculator.observed).len(), 1);
}

fn event_field<'object>(object: &'object Json, name: &str) -> Option<&'object Json> {
    let Json::Object(fields) = object else {
        return None;
    };
    fields.get(name)
}

#[test]
fn disabled_injection_returns_the_input_and_prices_nothing() {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, false);
    let event = json(CHAT_EVENT);
    assert!(matches!(
        injector.process_object(&event, "gpt-4o", None),
        Cow::Borrowed(_)
    ));
    let source = format!("data: {CHAT_EVENT}\n\n");
    assert!(matches!(
        injector.process_text(&source, "gpt-4o", None),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        injector.process_bytes(source.as_bytes(), "gpt-4o", None),
        Cow::Borrowed(_)
    ));
    assert!(take_calls(&recorder.observed).is_empty());
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn openai_zero_defaults_and_details_reach_the_recorder() {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(
        r#"{"object":"chat.completion.chunk","usage":{"prompt_tokens":7,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":4,"text_tokens":3},"completion_tokens_details":{"reasoning_tokens":1,"text_tokens":2}}}"#,
    );
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("0.5".to_owned()));
    let seen = take_calls(&recorder.observed);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model, "deployment");
    assert_eq!(seen[0].prompt_tokens, 7);
    assert_eq!(seen[0].completion_tokens, 3);
    assert_eq!(seen[0].total_tokens, 10);
    assert_eq!(
        seen[0].prompt_details,
        Some(details_object(r#"{"text_tokens":3,"cached_tokens":4}"#))
    );
    assert_eq!(
        seen[0].completion_details,
        Some(details_object(r#"{"text_tokens":2,"reasoning_tokens":1}"#))
    );
}

#[test]
fn openai_missing_counts_default_to_zero() {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(r#"{"object":"chat.completion.chunk","usage":{}}"#);
    let mut log = call_log("deployment");
    drop(injector.process_object(&event, "requested", Some(&mut log)));
    let seen = take_calls(&recorder.observed);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].prompt_tokens, 0);
    assert_eq!(seen[0].completion_tokens, 0);
    assert_eq!(seen[0].total_tokens, 0);
}

#[rstest]
#[case::cache_read_and_creation(true)]
#[case::plain_counts(false)]
fn anthropic_cache_usage_reaches_the_recorder(#[case] cached: bool) {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = if cached {
        json(DELTA_EVENT)
    } else {
        json(r#"{"type":"message_delta","usage":{"input_tokens":10,"output_tokens":5}}"#)
    };
    let mut log = call_log("claude");
    let output = injector.process_object(&event, "claude", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("0.5".to_owned()));
    let seen = take_calls(&recorder.observed);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].completion_tokens, 5);
    if cached {
        assert_eq!(seen[0].prompt_tokens, 15);
        assert_eq!(seen[0].total_tokens, 20);
        assert_eq!(
            seen[0].prompt_details,
            Some(details_object(
                r#"{"text_tokens":10,"cached_tokens":3,"cache_creation_tokens":2,"cache_write_tokens":2,"cache_creation_token_details":{"ephemeral_5m_input_tokens":1,"ephemeral_1h_input_tokens":1}}"#
            ))
        );
    } else {
        assert_eq!(seen[0].prompt_tokens, 10);
        assert_eq!(seen[0].total_tokens, 15);
    }
}

#[test]
fn the_anthropic_type_wins_over_the_chat_object() {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(
        r#"{"type":"message_delta","object":"chat.completion.chunk","usage":{"input_tokens":4,"output_tokens":1}}"#,
    );
    let mut log = call_log("claude");
    drop(injector.process_object(&event, "claude", Some(&mut log)));
    let seen = take_calls(&recorder.observed);
    assert_eq!(seen[0].prompt_tokens, 4);
    assert_eq!(seen[0].total_tokens, 5);
}

#[test]
fn a_recorder_zero_wins_over_the_fallback() {
    let recorder = recorder(Some("0"));
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("0.0".to_owned()));
    assert_eq!(take_calls(&recorder.observed).len(), 1);
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_missing_cost_falls_back_with_the_model_name_and_service_tier() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.25"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(
        r#"{"object":"chat.completion.chunk","service_tier":"priority","usage":{"prompt_tokens":2}}"#,
    );
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("0.25".to_owned()));
    assert_eq!(take_calls(&recorder.observed)[0].model, "deployment");
    let seen = take_calls(&calculator.observed);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model, "requested");
    assert_eq!(seen[0].service_tier.as_deref(), Some("priority"));
}

#[test]
fn both_pricing_attempts_failing_leave_the_object_unchanged() {
    let recorder = recorder(None);
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert!(matches!(output, Cow::Borrowed(_)));
    assert_eq!(output.as_ref(), &event);
}

#[test]
fn the_breakdown_and_the_diagnostic_are_restored() {
    let mut recorder = recorder(Some("0.5"));
    recorder.breakdown = Some(breakdown("9.5"));
    recorder.debug = Some(json(r#"{"written":true}"#));
    let calculator = calculator(Some("1.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    log.cost_breakdown = Some(breakdown("1.5"));
    log.model_call_details
        .insert(RESPONSE_COST_DEBUG.to_owned(), json(r#"{"seeded":true}"#));
    drop(injector.process_object(&event, "requested", Some(&mut log)));
    assert_eq!(log.cost_breakdown, Some(breakdown("1.5")));
    assert_eq!(
        log.model_call_details.get(RESPONSE_COST_DEBUG),
        Some(&json(r#"{"seeded":true}"#))
    );
}

#[test]
fn an_absent_diagnostic_stays_absent() {
    let mut recorder = recorder(None);
    recorder.breakdown = Some(breakdown("9.5"));
    recorder.debug = Some(json(r#"{"written":true}"#));
    let calculator = calculator(Some("1.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    log.cost_breakdown = Some(breakdown("1.5"));
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert!(cost_text(output.as_ref()).is_some());
    assert_eq!(log.cost_breakdown, Some(breakdown("1.5")));
    assert!(!log.model_call_details.contains_key(RESPONSE_COST_DEBUG));
}

#[test]
fn an_oversized_text_frame_is_not_inspected() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let pad = "x".repeat(MAX_INSPECTED_BYTES);
    let source = format!(
        "data: {{\"object\":\"chat.completion.chunk\",\"usage\":{{}},\"pad\":\"{pad}\"}}\n\n"
    );
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(matches!(output, Cow::Borrowed(_)));
    assert_eq!(output.len(), source.len());
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn an_oversized_byte_frame_is_not_decoded() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let pad = "x".repeat(MAX_INSPECTED_BYTES);
    let source = format!(
        "data: {{\"object\":\"chat.completion.chunk\",\"usage\":{{}},\"pad\":\"{pad}\"}}\r\n\r\n"
    );
    let output = injector.process_bytes(source.as_bytes(), "gpt-4o", None);
    assert!(matches!(output, Cow::Borrowed(_)));
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_text_frame_of_exactly_the_bound_is_inspected() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let pad = "x".repeat(MAX_INSPECTED_BYTES - EMPTY_FRAME_LEN);
    let source = format!(
        "data: {{\"object\":\"chat.completion.chunk\",\"usage\":{{}},\"pad\":\"{pad}\"}}\n\n"
    );
    assert_eq!(source.len(), MAX_INSPECTED_BYTES);
    let output = injector.process_text(&source, "gpt-4o", None);
    assert!(output.contains("\"cost\":0.5"));
}

#[test]
fn a_byte_frame_of_exactly_the_bound_is_decoded() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let pad = "x".repeat(MAX_INSPECTED_BYTES - EMPTY_FRAME_LEN);
    let source = format!(
        "data: {{\"object\":\"chat.completion.chunk\",\"usage\":{{}},\"pad\":\"{pad}\"}}\n\n"
    );
    assert_eq!(source.len(), MAX_INSPECTED_BYTES);
    let output = injector.process_bytes(source.as_bytes(), "gpt-4o", None);
    assert!(from_utf8(&output).unwrap().contains("\"cost\":0.5"));
}

#[test]
fn the_decoder_skips_a_frame_past_the_bound() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let pad = "x".repeat(MAX_INSPECTED_BYTES);
    let source = format!(
        "data: {{\"object\":\"chat.completion.chunk\",\"usage\":{{}},\"pad\":\"{pad}\"}}\n\n"
    );
    let output = decode_cost_frame(injector, source.as_bytes(), "gpt-4o", None);
    assert!(matches!(output, Cow::Borrowed(_)));
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn the_fixed_helpers_price_a_frame_with_a_nonzero_cost() {
    let recorder_priced = FixedRecorder(Some(amount("0.5")));
    let calculator_absent = FixedCalculator(None);
    let priced = StreamCostInjector {
        enabled: true,
        recorder: &recorder_priced,
        calculator: &calculator_absent,
    };
    let recorded_event = json(CHAT_EVENT);
    let recorded = priced.process_object(&recorded_event, "gpt-4o", None);
    assert_eq!(cost_text(recorded.as_ref()), Some("0.5".to_owned()));
    let recorder_absent = FixedRecorder(None);
    let calculator_priced = FixedCalculator(Some(amount("0.5")));
    let fallback = StreamCostInjector {
        enabled: true,
        recorder: &recorder_absent,
        calculator: &calculator_priced,
    };
    let computed_event = json(CHAT_EVENT);
    let computed = fallback.process_object(&computed_event, "gpt-4o", None);
    assert_eq!(cost_text(computed.as_ref()), Some("0.5".to_owned()));
}

#[test]
fn the_anthropic_counter_refuses_reasoning_text() {
    let counter = UnusedReasoningCounter;
    assert!(counter.count_reasoning_tokens("thinking about it").is_err());
}

#[test]
fn invalid_utf8_bytes_are_returned_unchanged() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = b"data: \xff\xfe\n\n".to_vec();
    let output = injector.process_bytes(&source, "gpt-4o", None);
    assert!(matches!(output, Cow::Borrowed(_)));
    assert_eq!(output.as_ref(), source.as_slice());
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_frame_without_a_data_line_is_unchanged() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let source = ": keep-alive\n\nevent: ping\n\n";
    let output = injector.process_text(source, "gpt-4o", None);
    assert_eq!(output.as_ref(), source);
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_non_object_usage_is_not_injectable() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(r#"{"object":"chat.completion.chunk","usage":5}"#);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert!(matches!(output, Cow::Borrowed(_)));
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_wrongly_typed_count_aborts_the_attempt() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(r#"{"object":"chat.completion.chunk","usage":{"prompt_tokens":"two"}}"#);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert!(matches!(output, Cow::Borrowed(_)));
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_wrongly_typed_details_field_aborts_the_attempt() {
    let recorder = recorder(None);
    let calculator = calculator(Some("0.5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(r#"{"object":"chat.completion.chunk","usage":{"prompt_tokens_details":3}}"#);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert!(matches!(output, Cow::Borrowed(_)));
    assert!(take_calls(&calculator.observed).is_empty());
}

#[test]
fn a_zero_cost_is_written_with_one_decimal_place() {
    let recorder = recorder(Some("0"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("0.0".to_owned()));
}

#[test]
fn a_whole_charge_from_the_fallback_keeps_one_decimal_place() {
    let recorder = recorder(None);
    let calculator = calculator(Some("5"));
    let injector = injector(&recorder, &calculator, true);
    let event = json(CHAT_EVENT);
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    assert_eq!(cost_text(output.as_ref()), Some("5.0".to_owned()));
}

#[test]
fn the_cost_key_keeps_its_place_after_the_other_usage_fields() {
    let recorder = recorder(Some("0.5"));
    let calculator = calculator(None);
    let injector = injector(&recorder, &calculator, true);
    let event = json(
        r#"{"object":"chat.completion.chunk","usage":{"prompt_tokens":2,"cost":1,"completion_tokens":1}}"#,
    );
    let mut log = call_log("deployment");
    let output = injector.process_object(&event, "requested", Some(&mut log));
    let Json::Object(fields) = output.as_ref() else {
        panic!("the object stays an object");
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        panic!("the usage stays an object");
    };
    let names: Vec<&str> = usage.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["prompt_tokens", "cost", "completion_tokens"]);
    assert_eq!(cost_text(output.as_ref()), Some("0.5".to_owned()));
}
