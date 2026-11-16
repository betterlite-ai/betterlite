#![cfg(test)]

use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use rstest::rstest;

use super::write_ended_tool_rewrites;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{chunks, response};
use crate::guardrails::chat_guardrail_translation::rewrites::tool_call_shapes::{
    ToolCallShape, function_tool_call_shapes,
};

const STREAM: &str = r#"[
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "a", "function": {"name": "f", "arguments": ""}}]}}]},
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"x\":"}}]}}]},
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "1}"}}]}}]},
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 1, "id": "b", "function": {"name": "g", "arguments": "{}"}}]}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}]"#;

const REBUILT: &str = r#"{"choices": [{"index": 0, "message": {"tool_calls": [
    {"id": "a", "function": {"name": "f", "arguments": "{\"x\":1}"}},
    {"id": "b", "function": {"name": "g", "arguments": "{}"}}]}}]}"#;

fn shapes_of(source: &str) -> Vec<ToolCallShape> {
    function_tool_call_shapes(&response(source))
}

fn refusal(reason: &str) -> UndeliverableStreamRewrite {
    UndeliverableStreamRewrite {
        guardrail_name: "pii".to_owned(),
        reason: reason.to_owned(),
    }
}

#[test]
fn write_tool_rewrites_put_the_name_and_arguments_in_the_first_fragment_and_blank_the_rest() {
    let mut stream = chunks(STREAM);
    let guardrailed = response(
        &REBUILT
            .replace(r#""name": "f""#, r#""name": "renamed""#)
            .replace(r#"{\"x\":1}"#, "{}"),
    );
    write_ended_tool_rewrites(&mut stream, &guardrailed, &shapes_of(REBUILT), "pii").unwrap();
    let expected = STREAM
        .replace(
            r#""name": "f", "arguments": """#,
            r#""name": "renamed", "arguments": "{}""#,
        )
        .replace(r#"{"arguments": "{\"x\":"}"#, r#"{"arguments": ""}"#)
        .replace(r#"{"arguments": "1}"}"#, r#"{"arguments": ""}"#);
    assert_eq!(stream, chunks(&expected));
}

#[test]
fn write_tool_rewrites_only_touch_the_calls_that_changed() {
    let mut stream = chunks(STREAM);
    let guardrailed = response(&REBUILT.replace(
        r#""name": "g", "arguments": "{}""#,
        r#""name": "g", "arguments": "{\"y\":2}""#,
    ));
    write_ended_tool_rewrites(&mut stream, &guardrailed, &shapes_of(REBUILT), "pii").unwrap();
    let expected = STREAM.replace(
        r#""name": "g", "arguments": "{}""#,
        r#""name": "g", "arguments": "{\"y\":2}""#,
    );
    assert_eq!(stream, chunks(&expected));
}

#[test]
fn write_tool_rewrites_do_nothing_when_the_calls_are_unchanged() {
    let mut stream = chunks(STREAM);
    write_ended_tool_rewrites(&mut stream, &response(REBUILT), &shapes_of(REBUILT), "pii").unwrap();
    assert_eq!(stream, chunks(STREAM));
}

#[test]
fn write_tool_rewrites_set_a_null_name_when_the_guardrail_removed_it() {
    let mut stream = chunks(STREAM);
    let guardrailed = response(&REBUILT.replace(r#""name": "g""#, r#""name": null"#));
    write_ended_tool_rewrites(&mut stream, &guardrailed, &shapes_of(REBUILT), "pii").unwrap();
    let expected = STREAM.replace(r#""name": "g""#, r#""name": null"#);
    assert_eq!(stream, chunks(&expected));
}

#[test]
fn write_tool_rewrites_refuse_a_stream_with_more_than_one_choice() {
    let two_choices = STREAM.replace(
        r#"{"index": 0, "delta": {}, "finish_reason": "tool_calls"}"#,
        r#"{"index": 1, "delta": {}}"#,
    );
    let mut stream = chunks(&two_choices);
    let guardrailed = response(&REBUILT.replace(r#""name": "f""#, r#""name": "renamed""#));
    let failure = write_ended_tool_rewrites(&mut stream, &guardrailed, &shapes_of(REBUILT), "pii")
        .unwrap_err();
    assert_eq!(
        failure,
        refusal(
            "the stream carries 2 choices and tool-call rewrites are only written back on single-choice streams"
        )
    );
    assert_eq!(stream, chunks(&two_choices));
}

#[test]
fn write_tool_rewrites_refuse_a_stream_with_no_choice() {
    let mut stream = chunks(r#"[{"model": "m"}]"#);
    let guardrailed = response(&REBUILT.replace(r#""name": "f""#, r#""name": "renamed""#));
    let failure = write_ended_tool_rewrites(&mut stream, &guardrailed, &shapes_of(REBUILT), "pii")
        .unwrap_err();
    assert!(failure.reason.starts_with("the stream carries 0 choices"));
}

#[rstest]
#[case::more_calls(
    r#"{"choices": [{"index": 0, "message": {"tool_calls": [
        {"function": {"name": "f", "arguments": "{}"}}, {"function": {"name": "g", "arguments": "{}"}},
        {"function": {"name": "h", "arguments": "{}"}}]}}]}"#,
    "the guardrail returned 3 tool calls for a stream that carried 2"
)]
#[case::fewer_calls(
    r#"{"choices": [{"index": 0, "message": {"tool_calls": [{"function": {"name": "f", "arguments": "{}"}}]}}]}"#,
    "the guardrail returned 1 tool calls for a stream that carried 2"
)]
fn write_tool_rewrites_refuse_calls_that_do_not_line_up_with_the_fragments(
    #[case] guardrailed: &str,
    #[case] reason: &str,
) {
    let mut stream = chunks(STREAM);
    let failure = write_ended_tool_rewrites(
        &mut stream,
        &response(guardrailed),
        &shapes_of(REBUILT),
        "pii",
    )
    .unwrap_err();
    assert_eq!(failure, refusal(reason));
    assert_eq!(stream, chunks(STREAM));
}
