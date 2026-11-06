#![cfg(test)]

use super::{MAX_RESPONSE_BYTES, ResponseToolCallError, parse_tool_call_response};
use crate::response_tool_calls::normalized::get_tool_calls_from_response;
use rstest::rstest;
use std::{fs, path::PathBuf};

fn call_count(body: &str) -> usize {
    let response = parse_tool_call_response(body.as_bytes()).expect("parsable response");
    get_tool_calls_from_response(&response, false).len()
}

#[rstest]
#[case::chat_section(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"a","function":{"name":"n","arguments":"{}"}}]}}]}"#,
    1
)]
#[case::unknown_envelope_fields_are_ignored(
    r#"{"id":"chat","model":"gpt-4","choices":[],"usage":{"prompt_tokens":1}}"#,
    0
)]
#[case::output_section(
    r#"{"output":[{"type":"function_call","call_id":"c","name":"n","arguments":"{}"}]}"#,
    1
)]
#[case::content_section(
    r#"{"content":[{"type":"tool_use","id":"t","name":"n","input":{}}]}"#,
    1
)]
#[case::empty_body_object("{}", 0)]
#[case::null_tool_calls_falls_through_to_output(
    r#"{"choices":[{"message":{"role":"assistant","content":"hi","tool_calls":null}}],"output":[{"type":"function_call","call_id":"c","name":"n","arguments":"{}"}]}"#,
    1
)]
#[case::null_choices_falls_through_to_output(
    r#"{"choices":null,"output":[{"type":"function_call","call_id":"c","name":"n","arguments":"{}"}]}"#,
    1
)]
#[case::null_output_falls_through_to_content(
    r#"{"choices":[],"output":null,"content":[{"type":"tool_use","id":"t","name":"n","input":{}}]}"#,
    1
)]
#[case::null_content_yields_no_calls(r#"{"choices":[],"output":[],"content":null}"#, 0)]
fn response_tool_calls_behavior_tests_behavior(#[case] body: &str, #[case] expected: usize) {
    assert_eq!(call_count(body), expected);
}

#[rstest]
#[case::choices_must_be_a_list(r#"{"choices":{}}"#)]
#[case::tool_calls_must_be_a_list(r#"{"choices":[{"message":{"tool_calls":{}}}]}"#)]
#[case::message_must_be_an_object(r#"{"choices":[{"message":"assistant"}]}"#)]
#[case::function_must_be_an_object(
    r#"{"choices":[{"message":{"tool_calls":[{"id":"a","function":"name"}]}}]}"#
)]
#[case::output_must_be_a_list(r#"{"output":"function_call"}"#)]
#[case::content_must_be_a_list(r#"{"content":"tool_use"}"#)]
#[case::invalid_json_is_an_error("{not json")]
#[case::trailing_bytes_are_an_error("{} trailing")]
fn response_tool_calls_parse_failures_behavior(#[case] body: &str) {
    assert!(matches!(
        parse_tool_call_response(body.as_bytes()),
        Err(ResponseToolCallError::Envelope { .. })
    ));
}

#[test]
fn response_tool_calls_rejects_oversized_bodies() {
    let body = vec![b' '; MAX_RESPONSE_BYTES.saturating_add(1)];
    let parsed = parse_tool_call_response(&body);
    assert!(matches!(
        parsed,
        Err(ResponseToolCallError::TooLarge { length, limit })
            if length == MAX_RESPONSE_BYTES.saturating_add(1) && limit == MAX_RESPONSE_BYTES
    ));
}

#[rstest]
fn replays_the_response_tool_calls_corpus(
    #[files("fuzz/corpus/response_tool_calls/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    if let Ok(response) = parse_tool_call_response(&bytes) {
        drop(get_tool_calls_from_response(&response, false));
        drop(get_tool_calls_from_response(&response, true));
    }
}
