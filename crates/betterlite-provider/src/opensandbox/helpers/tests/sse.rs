use rstest::rstest;
use serde_json::value::RawValue;

use super::super::sse::{
    Message, as_int, normalize_error, normalize_result, parse_lines, parse_sse_line,
};

fn message(text: &str) -> Message {
    serde_json::from_str(text).unwrap()
}

fn lines(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|line| (*line).to_owned()).collect()
}

fn raw_text(value: &RawValue) -> String {
    value.get().to_owned()
}

#[rstest]
#[case::empty("")]
#[case::whitespace("   ")]
#[case::comment(": keep-alive")]
#[case::event("event: message")]
#[case::identifier("id: 7")]
#[case::retry("retry: 1000")]
#[case::empty_data("data:")]
#[case::blank_data("data:   ")]
fn parse_sse_line_ignores_blank_comment_and_control_lines(#[case] line: &str) {
    assert!(parse_sse_line(line).is_none());
}

#[test]
fn parse_sse_line_reads_the_data_prefix() {
    let parsed = parse_sse_line("  data: {\"type\":\"stdout\",\"text\":\"hi\"}  ").unwrap();
    assert_eq!(raw_text(&parsed["type"]), "\"stdout\"");
    assert_eq!(raw_text(&parsed["text"]), "\"hi\"");
}

#[test]
fn parse_sse_line_reads_a_bare_json_object() {
    let parsed = parse_sse_line("{\"type\":\"stderr\"}").unwrap();
    assert_eq!(raw_text(&parsed["type"]), "\"stderr\"");
}

#[rstest]
#[case::truncated("{")]
#[case::array("[1,2]")]
#[case::string("\"text\"")]
#[case::number("5")]
#[case::null("null")]
fn parse_sse_line_skips_malformed_or_non_object_json(#[case] line: &str) {
    assert!(parse_sse_line(line).is_none());
}

#[test]
fn parse_sse_line_converts_an_untyped_code_and_message_error() {
    let parsed = parse_sse_line(r#"{"code":"invalid_request","message":"bad code"}"#).unwrap();
    assert_eq!(raw_text(&parsed["type"]), "\"error\"");
    let error = &parsed["error"];
    let fields: Message = serde_json::from_str(error.get()).unwrap();
    assert_eq!(raw_text(&fields["ename"]), "\"invalid_request\"");
    assert_eq!(raw_text(&fields["evalue"]), "\"bad code\"");
    assert_eq!(raw_text(&fields["traceback"]), "[]");
}

#[test]
fn parse_sse_line_keeps_a_typed_message_unchanged() {
    let parsed = parse_sse_line(r#"{"type":"error","code":"x","message":"y"}"#).unwrap();
    assert!(!parsed.contains_key("error"));
    assert_eq!(raw_text(&parsed["code"]), "\"x\"");
}

#[rstest]
#[case::code_only(r#"{"code":"x"}"#)]
#[case::message_only(r#"{"message":"y"}"#)]
fn parse_sse_line_keeps_a_partial_error_shape(#[case] line: &str) {
    let parsed = parse_sse_line(line).unwrap();
    assert!(!parsed.contains_key("type"));
    assert!(!parsed.contains_key("error"));
}

#[test]
fn parse_lines_concatenates_stdout_and_stderr_in_encounter_order() {
    let result = parse_lines(&lines(&[
        r#"{"type":"stdout","text":"a"}"#,
        r#"{"type":"stderr","text":"e1"}"#,
        r#"{"type":"stdout","text":"b"}"#,
        r#"{"type":"stderr","text":"e2"}"#,
    ]));
    assert_eq!(result.stdout, "ab");
    assert_eq!(result.stderr, "e1e2");
}

#[test]
fn parse_lines_takes_the_first_error_and_the_first_parsed_count() {
    let result = parse_lines(&lines(&[
        r#"{"type":"error","error":{"ename":"First","evalue":"one"}}"#,
        r#"{"type":"error","error":{"ename":"Second","evalue":"two"}}"#,
        r#"{"type":"execution_count","execution_count":"3"}"#,
        r#"{"type":"execution_count","execution_count":4}"#,
    ]));
    let error = result.error.unwrap();
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"First\"");
    assert_eq!(result.execution_count, Some(3));
}

#[test]
fn parse_lines_skips_a_count_that_is_not_a_number() {
    let result = parse_lines(&lines(&[
        r#"{"type":"execution_count","execution_count":"many"}"#,
        r#"{"type":"execution_count","execution_count":7}"#,
    ]));
    assert_eq!(result.execution_count, Some(7));
}

#[test]
fn parse_lines_ignores_lines_that_do_not_parse() {
    let result = parse_lines(&lines(&[
        "not json",
        "",
        "data:",
        r#"{"type":"stdout","text":"x"}"#,
    ]));
    assert_eq!(result.stdout, "x");
}

#[test]
fn parse_lines_collects_result_messages_with_the_envelope_removed() {
    let result = parse_lines(&lines(&[
        r#"{"type":"result","text":"7","timestamp":1,"execution_count":2}"#,
        r#"{"type":"result","results":{"text/plain":"hi","image/png":"AA=="}}"#,
    ]));
    assert_eq!(result.results.len(), 2);
    assert_eq!(
        result.results[0].keys().cloned().collect::<Vec<_>>(),
        vec!["text".to_owned()]
    );
    assert_eq!(raw_text(&result.results[1]["text/plain"]), "\"hi\"");
    assert_eq!(raw_text(&result.results[1]["image/png"]), "\"AA==\"");
}

#[test]
fn normalize_result_copies_non_result_fields_when_results_is_not_an_object() {
    let parsed = message(r#"{"results":["a"],"text":"body"}"#);
    let normalized = normalize_result(&parsed);
    assert_eq!(normalized.len(), 2);
    assert_eq!(raw_text(&normalized["results"]), "[\"a\"]");
}

#[test]
fn normalize_error_reads_a_nested_error() {
    let parsed =
        message(r#"{"type":"error","error":{"ename":"E","evalue":"V","traceback":["t"]}}"#);
    let error = normalize_error(&parsed);
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"E\"");
    assert_eq!(raw_text(error.value.as_ref().unwrap()), "\"V\"");
    assert_eq!(raw_text(error.traceback.as_ref().unwrap()), "[\"t\"]");
}

#[test]
fn normalize_error_falls_back_to_alternate_nested_keys() {
    let parsed = message(r#"{"type":"error","error":{"name":"E","value":"V"}}"#);
    let error = normalize_error(&parsed);
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"E\"");
    assert_eq!(raw_text(error.value.as_ref().unwrap()), "\"V\"");
    assert_eq!(raw_text(error.traceback.as_ref().unwrap()), "[]");
}

#[test]
fn normalize_error_skips_nested_null_fields() {
    let parsed = message(r#"{"type":"error","error":{"ename":null,"name":"E","evalue":null}}"#);
    let error = normalize_error(&parsed);
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"E\"");
    assert_eq!(raw_text(error.value.as_ref().unwrap()), "\"\"");
}

#[test]
fn normalize_error_reads_top_level_fields_when_the_error_is_not_nested() {
    let parsed = message(r#"{"type":"error","name":"E","text":"T"}"#);
    let error = normalize_error(&parsed);
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"E\"");
    assert_eq!(raw_text(error.value.as_ref().unwrap()), "\"T\"");
    assert_eq!(raw_text(error.traceback.as_ref().unwrap()), "[]");
}

#[test]
fn normalize_error_defaults_every_field_when_the_message_is_bare() {
    let parsed = message(r#"{"type":"error"}"#);
    let error = normalize_error(&parsed);
    assert_eq!(raw_text(error.name.as_ref().unwrap()), "\"\"");
    assert_eq!(raw_text(error.value.as_ref().unwrap()), "\"\"");
    assert_eq!(raw_text(error.traceback.as_ref().unwrap()), "[]");
}

#[rstest]
#[case::number("5", Some(5))]
#[case::negative("-3", Some(-3))]
#[case::string_integer("\"12\"", Some(12))]
#[case::negative_string("\"-3\"", Some(-3))]
#[case::float("5.5", None)]
#[case::text("\"many\"", None)]
#[case::boolean("true", None)]
#[case::null("null", None)]
fn as_int_reads_integers_and_integer_strings(#[case] text: &str, #[case] expected: Option<i64>) {
    let raw: Box<RawValue> = serde_json::from_str(text).unwrap();
    assert_eq!(as_int(&raw), expected);
}
