#![cfg(test)]

use std::fs;
use std::path::Path;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{
    ResponsesSseRecoveryError, decode_recovery_line, recover_logged_output, recover_recorded_output,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn list(text: &str) -> Vec<Json> {
    serde_json::from_str(text).expect("the fixture is a JSON array")
}

fn decode(input: &str) -> Option<IndexMap<String, Json>> {
    decode_recovery_line(input.as_bytes(), &ApiLimits::default()).expect("the line decodes")
}

fn recover(input: &str) -> Vec<Json> {
    recover_recorded_output(input.as_bytes(), &ApiLimits::default()).expect("the recording decodes")
}

fn item_limits(max_items: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items,
    }
}

fn event_limits(max_event_bytes: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes,
        max_items: 64 * 1024,
    }
}

fn recording<E>(events: &[E]) -> String
where
    E: AsRef<str>,
{
    events
        .iter()
        .map(|event| format!("data: {}", event.as_ref()))
        .collect::<Vec<String>>()
        .join("\n")
}

fn item(index: i64, id: &str) -> String {
    format!(
        r#"{{"type":"response.output_item.done","output_index":{index},"item":{{"id":"{id}"}}}}"#
    )
}

fn completed(output: &str) -> String {
    format!(r#"{{"type":"response.completed","response":{{"output":{output}}}}}"#)
}

fn text_done(index: i64, text: &str) -> String {
    format!(r#"{{"type":"response.output_text.done","output_index":{index},"text":"{text}"}}"#)
}

fn text_at(index: i64, content: i64, text: &str) -> String {
    format!(
        r#"{{"type":"response.output_text.done","output_index":{index},"content_index":{content},"text":"{text}"}}"#
    )
}

fn message(index: i64, text: &str) -> Json {
    json(&format!(
        r#"{{"type":"message","id":"msg_{index}","role":"assistant","status":"completed","content":[{{"type":"output_text","text":"{text}","annotations":[]}}]}}"#
    ))
}

fn message_with(id: &str, text: &str, annotations: &str) -> Json {
    json(&format!(
        r#"{{"type":"message","id":{id},"role":"assistant","status":"completed","content":[{{"type":"output_text","text":"{text}","annotations":{annotations}}}]}}"#
    ))
}

fn state() -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "responses".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .expect("the fixture is RFC 3339"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
        messages: None,
        stream: Some(true),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
        || "trace-1".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}

#[rstest]
#[case::plain_object(r#"{"a": 1}"#, Some(r#"{"a": 1}"#))]
#[case::surrounding_whitespace("  {\"a\": 1}\t", Some(r#"{"a": 1}"#))]
#[case::data_prefix(r#"data: {"a": 1}"#, Some(r#"{"a": 1}"#))]
#[case::data_prefix_without_space(r#"data:{"a": 1}"#, Some(r#"{"a": 1}"#))]
#[case::payload_trailing_space("data: {\"a\": 1}  ", Some(r#"{"a": 1}"#))]
#[case::empty_object("data: {}", Some("{}"))]
#[case::empty_line("", None)]
#[case::blank_line("   ", None)]
#[case::done_marker("data: [DONE]", None)]
#[case::bare_done_marker("[DONE]", None)]
#[case::event_line("event: response.completed", None)]
#[case::data_event_line("data: event: response.completed", None)]
#[case::malformed_json("data: {\"a\":", None)]
#[case::array_payload("data: [1, 2]", None)]
#[case::number_payload("data: 5", None)]
#[case::string_payload(r#"data: "text""#, None)]
#[case::null_payload("data: null", None)]
#[case::boolean_payload("data: true", None)]
#[case::embedded_prefix(r#"text data: {"a": 1}"#, None)]
#[case::double_prefix(r#"data:data:{"a": 1}"#, None)]
#[case::trailing_garbage(r#"data: {"a": 1} trailing"#, None)]
#[case::empty_data("data:", None)]
fn recovery_line_filter(#[case] input: &str, #[case] expected: Option<&str>) {
    assert_eq!(decode(input).map(Json::Object), expected.map(json));
}

#[rstest]
#[case::non_breaking_space("\u{a0}{\"a\": 1}\u{a0}", Some(r#"{"a": 1}"#))]
#[case::file_separators("\u{1c}{\"a\": 1}\u{1f}", Some(r#"{"a": 1}"#))]
#[case::vertical_tab("\u{b}{\"a\": 1}\u{c}", Some(r#"{"a": 1}"#))]
#[case::line_separator("\u{2028}{\"a\": 1}\u{2029}", Some(r#"{"a": 1}"#))]
fn recovery_line_filter_strips_python_whitespace(
    #[case] input: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(decode(input).map(Json::Object), expected.map(json));
}

#[test]
fn recovery_line_filter_parses_a_multiline_payload() {
    assert_eq!(
        decode("data: {\n\"a\": 1}").map(Json::Object),
        Some(json(r#"{"a": 1}"#))
    );
}

#[test]
fn recovery_line_filter_does_not_join_data_lines() {
    let first = r#"data: {"type":"response.output_item.done","#;
    let second = r#"data: "output_index":0,"item":{"id":"x"}}"#;
    // joining the two payloads would parse as one output_item.done event and recover an item
    assert!(recover(&format!("{first}\n{second}")).is_empty());
}

#[rstest]
#[case::carriage_return_line_feed("\r\n")]
#[case::carriage_return("\r")]
#[case::vertical_tab("\u{b}")]
#[case::form_feed("\u{c}")]
#[case::file_separator("\u{1c}")]
#[case::group_separator("\u{1d}")]
#[case::record_separator("\u{1e}")]
#[case::next_line("\u{85}")]
#[case::line_separator("\u{2028}")]
#[case::paragraph_separator("\u{2029}")]
fn recovery_splits_python_line_boundaries(#[case] separator: &str) {
    let events = [item(0, "zero"), item(1, "one")].map(|event| format!("data: {event}"));
    assert_eq!(
        recover(&events.join(separator)),
        list(r#"[{"id":"zero"},{"id":"one"}]"#)
    );
}

#[test]
fn recovery_does_not_split_on_the_unit_separator() {
    // Python's splitlines leaves U+001F inside the line, so the joined payload is no JSON object
    let events = [item(0, "zero"), item(1, "one")].join("\u{1f}");
    assert!(recover(&events).is_empty());
}

#[test]
fn recovery_line_filter_rejects_a_line_over_the_event_cap() {
    let limits = event_limits(8);
    let error = decode_recovery_line(b"data: {\"a\": 1}", &limits).expect_err("the line is over");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::EventTooLarge {
            observed: 14,
            limit: 8
        })
    ));
    assert_eq!(
        decode_recovery_line(b"{\"a\": 1}", &limits).expect("the object line decodes"),
        Some(IndexMap::from([("a".to_owned(), json("1"))]))
    );
}

#[test]
fn recovery_rejects_bytes_that_are_not_utf8() {
    let bytes = b"data: \xff";
    let error = decode_recovery_line(bytes, &ApiLimits::default()).expect_err("the bytes are not");
    assert!(matches!(error, ResponsesSseRecoveryError::Utf8(_)));
    let recovered_error =
        recover_recorded_output(bytes, &ApiLimits::default()).expect_err("the bytes are not");
    assert!(matches!(
        recovered_error,
        ResponsesSseRecoveryError::Utf8(_)
    ));
}

#[test]
fn recovery_drops_nonstandard_json_literals() {
    let nan = r#"{"type":"response.output_item.done","output_index":NaN,"item":{"id":"nan"}}"#;
    let infinity =
        r#"{"type":"response.output_item.done","output_index":Infinity,"item":{"id":"beyond"}}"#;
    let surrogate = r#"{"type":"response.output_text.done","output_index":0,"text":"x","content_index":0,"item_id":"\ud800"}"#;
    let nest = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let deep = format!(
        r#"{{"type":"response.output_item.done","output_index":0,"item":{{"id":"deep","nested":{nest}}}}}"#
    );
    // Python's json.loads takes all four; RFC 8259 JSON does not, a nested line exceeds serde_json's 128-level cap, and an Infinity index raises OverflowError there
    let events = [nan, infinity, surrogate, deep.as_str()];
    assert!(recover(&recording(&events)).is_empty());
}

#[rstest]
#[case::string_with_spaces(r#""output_index": " +3 ","#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::underscored_string(r#""output_index": "1_000","#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::boolean_true(r#""output_index": true,"#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::truncated_float(r#""output_index": 2.9,"#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::negative(r#""output_index": -1,"#, r#"[{"id":"tested"},{"id":"zero"}]"#)]
#[case::absent("", r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::null(r#""output_index": null,"#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::invalid_string(r#""output_index": "abc","#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::float_string(r#""output_index": "2.9","#, r#"[{"id":"zero"},{"id":"tested"}]"#)]
#[case::boolean_false(r#""output_index": false,"#, r#"[{"id":"tested"}]"#)]
fn indexed_item_recovery(#[case] index: &str, #[case] expected: &str) {
    let first = item(0, "zero");
    let second =
        format!(r#"{{"type":"response.output_item.done",{index}"item":{{"id":"tested"}}}}"#);
    assert_eq!(
        recover(&recording(&[first.as_str(), second.as_str()])),
        list(expected)
    );
}

#[rstest]
#[case::missing_item(r#"{"type":"response.output_item.done","output_index":0}"#)]
#[case::number_item(r#"{"type":"response.output_item.done","output_index":0,"item":5}"#)]
#[case::null_item(r#"{"type":"response.output_item.done","output_index":0,"item":null}"#)]
#[case::array_item(r#"{"type":"response.output_item.done","output_index":0,"item":[]}"#)]
#[case::string_item(r#"{"type":"response.output_item.done","output_index":0,"item":"x"}"#)]
#[case::boolean_item(r#"{"type":"response.output_item.done","output_index":0,"item":true}"#)]
fn indexed_item_recovery_ignores_a_non_object_item(#[case] event: &str) {
    assert!(recover(&recording(&[event])).is_empty());
}

#[test]
fn indexed_item_recovery_repeated_index_overwrites() {
    let recorded = recording(&[item(0, "first"), item(0, "second")]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"second"}]"#));
}

#[test]
fn indexed_item_recovery_missing_index_overwrites_a_sparse_slot() {
    let recorded = recording(&[
        item(0, "zero"),
        item(2, "two"),
        r#"{"type":"response.output_item.done","item":{"id":"fallback"}}"#.to_owned(),
    ]);
    assert_eq!(
        recover(&recorded),
        list(r#"[{"id":"zero"},{"id":"fallback"}]"#)
    );
}

#[test]
fn indexed_item_recovery_rejects_more_items_than_the_cap() {
    let limits = item_limits(2);
    let recorded = recording(&[item(0, "a"), item(1, "b"), item(2, "c")]);
    let error =
        recover_recorded_output(recorded.as_bytes(), &limits).expect_err("the third item is over");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems {
            observed: 3,
            limit: 2
        })
    ));
}

#[test]
fn text_item_recovery() {
    assert_eq!(
        recover(&recording(&[text_done(0, "").as_str()])),
        vec![message(0, "")]
    );
}

fn coerced(index_field: &str, target: i64) -> String {
    let event =
        format!(r#"{{"type":"response.output_item.done",{index_field}"item":{{"id":"tested"}}}}"#);
    recording(&[event.as_str(), item(target, "target").as_str()])
}

#[rstest]
#[case::signed_string(r#""output_index": " +3 ","#, 3)]
#[case::underscored_string(r#""output_index": "1_000","#, 1000)]
#[case::boolean_true(r#""output_index": true,"#, 1)]
#[case::truncated_float(r#""output_index": 2.9,"#, 2)]
fn indexed_item_recovery_coerces_to_the_parsed_index(#[case] index: &str, #[case] target: i64) {
    // the target item overwrites the coerced one only when both land on the same index
    let expected = list(r#"[{"id":"target"}]"#);
    assert_eq!(recover(&coerced(index, target)), expected);
}

#[rstest]
#[case::invalid_string(r#""output_index": "abc","#)]
#[case::float_string(r#""output_index": "2.9","#)]
#[case::list(r#""output_index": [],"#)]
#[case::object(r#""output_index": {},"#)]
#[case::trailing_underscore(r#""output_index": "1_","#)]
#[case::leading_underscore(r#""output_index": "_1","#)]
#[case::double_underscore(r#""output_index": "1__2","#)]
fn indexed_item_recovery_falls_back_for_an_unusable_index(#[case] index: &str) {
    let expected = list(r#"[{"id":"tested"},{"id":"target"}]"#);
    assert_eq!(recover(&coerced(index, 1)), expected);
}

#[test]
fn indexed_item_recovery_bounds_an_index_to_i64() {
    let largest = r#"{"type":"response.output_item.done","output_index":9223372036854775807,"item":{"id":"max"}}"#;
    let beyond = r#"{"type":"response.output_item.done","output_index":9223372036854775808,"item":{"id":"beyond"}}"#;
    // LiteLLM keeps an index past i64 and sorts it last; here it falls back to the real-item count
    assert_eq!(
        recover(&recording(&[largest, beyond])),
        list(r#"[{"id":"beyond"},{"id":"max"}]"#)
    );
}

#[test]
fn indexed_item_recovery_ignores_a_non_ascii_digit_index() {
    // U+0663 is ARABIC-INDIC DIGIT THREE, which Python's int() reads as 3 and this code does not
    let index = "\"output_index\": \"\u{663}\",";
    assert_eq!(
        recover(&coerced(index, 1)),
        list(r#"[{"id":"tested"},{"id":"target"}]"#)
    );
}

#[rstest]
#[case::number(r#"{"type":"response.output_text.done","output_index":0,"text":5}"#)]
#[case::null(r#"{"type":"response.output_text.done","output_index":0,"text":null}"#)]
#[case::object(r#"{"type":"response.output_text.done","output_index":0,"text":{}}"#)]
#[case::array(r#"{"type":"response.output_text.done","output_index":0,"text":[]}"#)]
#[case::boolean(r#"{"type":"response.output_text.done","output_index":0,"text":true}"#)]
#[case::missing(r#"{"type":"response.output_text.done","output_index":0}"#)]
fn text_item_recovery_ignores_a_nonstring_text(#[case] event: &str) {
    assert!(recover(&recording(&[event])).is_empty());
}

#[rstest]
#[case::text_kept(r#""item_id":"custom","#, r#""custom""#)]
#[case::item_id_absent("", r#""msg_0""#)]
#[case::item_id_null(r#""item_id":null,"#, r#""msg_0""#)]
#[case::item_id_empty(r#""item_id":"","#, r#""msg_0""#)]
#[case::item_id_false(r#""item_id":false,"#, r#""msg_0""#)]
#[case::item_id_zero(r#""item_id":0,"#, r#""msg_0""#)]
#[case::item_id_number(r#""item_id":5,"#, "5")]
#[case::item_id_empty_list(r#""item_id":[],"#, r#""msg_0""#)]
#[case::item_id_empty_object(r#""item_id":{},"#, r#""msg_0""#)]
#[case::item_id_list(r#""item_id":[1],"#, "[1]")]
#[case::item_id_object(r#""item_id":{"a":1},"#, r#"{"a":1}"#)]
fn text_item_recovery_item_id(#[case] id_field: &str, #[case] expected: &str) {
    let event = format!(
        r#"{{"type":"response.output_text.done","output_index":0,{id_field}"text":"x","content_index":0}}"#
    );
    let recovered = recover(&recording(&[event.as_str()]));
    assert_eq!(recovered, vec![message_with(expected, "x", "[]")]);
}

#[rstest]
#[case::negative_index(-1_i64, 0_usize)]
#[case::past_the_cap(1025_i64, 0_usize)]
#[case::last_allowed(1024_i64, 1025_usize)]
#[case::largest_i64(i64::MAX, 0_usize)]
fn text_item_recovery_content_index_limit(#[case] index: i64, #[case] entries: usize) {
    let event = format!(
        r#"{{"type":"response.output_text.done","output_index":0,"text":"x","content_index":{index}}}"#
    );
    let recovered = recover(&recording(&[event.as_str()]));
    let [Json::Object(message)] = recovered.as_slice() else {
        panic!("the recording recovers one message");
    };
    let Some(Json::List(content)) = message.get("content") else {
        panic!("the message has content");
    };
    assert_eq!(content.len(), entries);
}

#[test]
fn text_item_recovery_pads_missing_content_indices() {
    let recorded = recording(&[text_at(0, 2, "two")]);
    let expected = list(
        r#"[{"type":"message","id":"msg_0","role":"assistant","status":"completed","content":[
            {"type":"output_text","text":"","annotations":[]},
            {"type":"output_text","text":"","annotations":[]},
            {"type":"output_text","text":"two","annotations":[]}]}]"#,
    );
    assert_eq!(recover(&recorded), expected);
}

#[test]
fn text_item_recovery_appends_without_a_content_index() {
    let recorded = recording(&[text_done(0, "first"), text_done(0, "second")]);
    let expected = list(
        r#"[{"type":"message","id":"msg_0","role":"assistant","status":"completed","content":[
            {"type":"output_text","text":"first","annotations":[]},
            {"type":"output_text","text":"second","annotations":[]}]}]"#,
    );
    assert_eq!(recover(&recorded), expected);
}

#[test]
fn text_item_recovery_replaces_text_at_a_repeated_index() {
    let recorded = recording(&[text_at(0, 0, "first"), text_at(0, 0, "second")]);
    assert_eq!(recover(&recorded), vec![message(0, "second")]);
}

#[rstest]
#[case::absent("", r#"[{"url":"u"}]"#)]
#[case::null(r#","annotations":null"#, r#"[{"url":"u"}]"#)]
#[case::replaced(r#","annotations":[{"url":"v"}]"#, r#"[{"url":"v"}]"#)]
fn text_item_recovery_annotations(#[case] field: &str, #[case] expected: &str) {
    let first = r#"{"type":"response.output_text.done","output_index":0,"content_index":0,"text":"a","annotations":[{"url":"u"}]}"#;
    let second = format!(
        r#"{{"type":"response.output_text.done","output_index":0,"content_index":0,"text":"b"{field}}}"#
    );
    let recorded = recording(&[first, second.as_str()]);
    assert_eq!(
        recover(&recorded),
        vec![message_with(r#""msg_0""#, "b", expected)]
    );
}

#[test]
fn text_item_recovery_real_item_suppresses_text_writes() {
    let text = text_done(0, "synthetic");
    let real = item(0, "real");
    assert_eq!(
        recover(&recording(&[text.as_str(), real.as_str()])),
        list(r#"[{"id":"real"}]"#)
    );
    assert_eq!(
        recover(&recording(&[real.as_str(), text.as_str()])),
        list(r#"[{"id":"real"}]"#)
    );
}

#[test]
fn text_item_recovery_fallback_counts_text_items() {
    let recorded = recording(&[
        item(5, "real"),
        r#"{"type":"response.output_text.done","text":"synthetic"}"#.to_owned(),
    ]);
    let expected = vec![message(0, "synthetic"), json(r#"{"id":"real"}"#)];
    assert_eq!(recover(&recorded), expected);
}

#[test]
fn text_item_recovery_rejects_content_over_the_item_cap() {
    let limits = item_limits(8);
    let recorded = recording(&[text_at(0, 10, "x")]);
    let error =
        recover_recorded_output(recorded.as_bytes(), &limits).expect_err("the padding is bounded");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems {
            observed: 9,
            limit: 8
        })
    ));
}

#[test]
fn text_item_recovery_rejects_more_items_than_the_cap() {
    let limits = item_limits(1);
    let recorded = recording(&[text_done(0, "zero"), text_done(1, "one")]);
    let error =
        recover_recorded_output(recorded.as_bytes(), &limits).expect_err("the second item is over");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems {
            observed: 2,
            limit: 1
        })
    ));
}

#[test]
fn terminal_output_precedence() {
    let done = completed(r#"[{"id":"done"}]"#);
    let early = item(0, "early");
    let later = item(0, "later");
    assert_eq!(
        recover(&recording(&[early.as_str(), done.as_str(), later.as_str()])),
        list(r#"[{"id":"done"}]"#)
    );
    let synthetic = text_done(0, "synthetic");
    let real = item(0, "real");
    assert_eq!(
        recover(&recording(&[synthetic.as_str(), real.as_str()])),
        list(r#"[{"id":"real"}]"#)
    );
    let delta = r#"{"type":"response.output_text.delta","output_index":0,"delta":"hello"}"#;
    assert!(recover(&recording(&[delta])).is_empty());
    let mut logging = state();
    let absent = recover_logged_output(&logging, &ApiLimits::default()).expect("the details");
    assert!(absent.is_empty());
    logging.model_call_details =
        IndexMap::from([("original_response".to_owned(), Json::Text(done))]);
    let logged = recover_logged_output(&logging, &ApiLimits::default()).expect("the details");
    assert_eq!(logged, list(r#"[{"id":"done"}]"#));
}

#[test]
fn terminal_output_precedence_completed_wins_immediately() {
    let recorded = recording(&[completed(r#"[{"id":"done"}]"#), item(0, "later")]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"done"}]"#));
}

#[test]
fn terminal_output_precedence_completed_replaces_recovered_items() {
    let recorded = recording(&[item(0, "early"), completed(r#"[{"id":"done"}]"#)]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"done"}]"#));
}

#[test]
fn terminal_output_precedence_completed_wins_over_a_later_completed() {
    let recorded = recording(&[
        completed(r#"[{"id":"first"}]"#),
        completed(r#"[{"id":"second"}]"#),
    ]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"first"}]"#));
}

#[test]
fn terminal_output_precedence_keeps_completed_output_verbatim() {
    let recorded = recording(&[completed(r#"[1, "two", null]"#)]);
    assert_eq!(recover(&recorded), list(r#"[1, "two", null]"#));
}

#[rstest]
#[case::missing_response(r#"{"type":"response.completed"}"#)]
#[case::non_object_response(r#"{"type":"response.completed","response":[]}"#)]
#[case::missing_output(r#"{"type":"response.completed","response":{}}"#)]
#[case::empty_output(r#"{"type":"response.completed","response":{"output":[]}}"#)]
#[case::non_list_output(r#"{"type":"response.completed","response":{"output":"x"}}"#)]
#[case::incomplete(r#"{"type":"response.incomplete","response":{"output":[{"id":"incomplete"}]}}"#)]
fn terminal_output_precedence_continues_past_a_nonterminal_event(#[case] event: &str) {
    let later = item(0, "recovered");
    assert_eq!(
        recover(&recording(&[event, later.as_str()])),
        list(r#"[{"id":"recovered"}]"#)
    );
    assert!(recover(&recording(&[event])).is_empty());
}

#[test]
fn terminal_output_precedence_real_item_wins_when_it_arrives_first() {
    let recorded = recording(&[item(0, "real"), text_done(0, "synthetic")]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"real"}]"#));
}

#[test]
fn terminal_output_precedence_real_item_wins_when_text_arrives_first() {
    let recorded = recording(&[text_done(0, "synthetic"), item(0, "real")]);
    assert_eq!(recover(&recorded), list(r#"[{"id":"real"}]"#));
}

#[test]
fn terminal_output_precedence_sorts_indices() {
    let recorded = recording(&[text_done(2, "two"), item(1, "one"), text_done(0, "zero")]);
    let expected = vec![
        message(0, "zero"),
        json(r#"{"id":"one"}"#),
        message(2, "two"),
    ];
    assert_eq!(recover(&recorded), expected);
}

#[test]
fn terminal_output_precedence_text_deltas_recover_nothing() {
    let delta = r#"{"type":"response.output_text.delta","output_index":0,"delta":"hello"}"#;
    let part = r#"{"type":"response.content_part.done","output_index":0,"part":{"type":"output_text","text":"hello"}}"#;
    assert!(recover(&recording(&[delta, part])).is_empty());
}

#[rstest]
#[case::absent(None, "[]")]
#[case::null(Some(Json::Null), "[]")]
#[case::number(Some(Json::Number(Number::from(5_u8))), "[]")]
#[case::list(Some(Json::List(Vec::new())), "[]")]
#[case::empty_recording(Some(Json::Text(String::new())), "[]")]
#[case::completed(
    Some(Json::Text(
        "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"id\":\"done\"}]}}"
            .to_owned()
    )),
    r#"[{"id":"done"}]"#
)]
fn terminal_output_precedence_logging(#[case] original: Option<Json>, #[case] expected: &str) {
    let mut details = IndexMap::new();
    if let Some(value) = original {
        details.insert("original_response".to_owned(), value);
    }
    let mut logging = state();
    logging.model_call_details = details;
    let recovered = recover_logged_output(&logging, &ApiLimits::default()).expect("the details");
    assert_eq!(recovered, list(expected));
}

#[test]
fn terminal_output_precedence_rejects_completed_output_over_the_cap() {
    let limits = item_limits(2);
    let recorded = recording(&[completed(r#"[{"id":"a"},{"id":"b"},{"id":"c"}]"#)]);
    let error =
        recover_recorded_output(recorded.as_bytes(), &limits).expect_err("the output is bounded");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems {
            observed: 3,
            limit: 2
        })
    ));
}

#[test]
fn terminal_output_precedence_rejects_a_merge_over_the_cap() {
    let limits = item_limits(3);
    let recorded = recording(&[
        text_done(0, "zero"),
        text_done(1, "one"),
        item(2, "two"),
        item(3, "three"),
    ]);
    let error =
        recover_recorded_output(recorded.as_bytes(), &limits).expect_err("the merge is bounded");
    assert!(matches!(
        error,
        ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems {
            observed: 4,
            limit: 3
        })
    ));
}

fn replay(seed: &[u8]) {
    let Some((selector, payload)) = seed.split_first() else {
        return;
    };
    if selector.is_multiple_of(2) {
        drop(decode_recovery_line(payload, &ApiLimits::default()));
        return;
    }
    drop(recover_recorded_output(payload, &ApiLimits::default()));
}

// every seed takes the path its first byte selects
fn replay_all(seeds: &[Vec<u8>]) {
    for seed in seeds {
        replay(seed);
    }
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_sse_recovery");
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| fs::read(entry.expect("an entry").path()).expect("a seed file"))
        .collect();
    assert!(seeds.len() >= 6);
    let selectors: Vec<u8> = seeds
        .iter()
        .filter_map(|seed| seed.first().copied())
        .collect();
    assert!(selectors.iter().any(|selector| selector.is_multiple_of(2)));
    assert!(selectors.iter().any(|selector| !selector.is_multiple_of(2)));
    replay_all(&seeds);
}
