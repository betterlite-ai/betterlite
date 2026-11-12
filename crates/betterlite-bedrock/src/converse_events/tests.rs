#![cfg(test)]

mod scenarios;
mod support;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::converse_events::projection::UnusedCounter;
use crate::converse_usage::transform::ReasoningTokenCounter as _;

use self::scenarios::{
    content_stop_finish, content_text_delta, converse_projection_error,
    invoke_chunk_provider_dispatch, json_tool_stream, message_start_id, metadata_usage,
    thinking_signature, tool_input_delta,
};
use self::support::{FIRST, event, state};

#[rstest]
#[case::message_start_id(message_start_id)]
#[case::content_text_delta(content_text_delta)]
#[case::tool_input_delta(tool_input_delta)]
#[case::thinking_signature(thinking_signature)]
#[case::content_stop_finish(content_stop_finish)]
#[case::metadata_usage(metadata_usage)]
#[case::invoke_chunk_provider_dispatch(invoke_chunk_provider_dispatch)]
#[case::json_tool_stream(json_tool_stream)]
#[case::converse_projection_error(converse_projection_error)]
fn converse_events_cases(#[case] case: fn()) {
    case();
}

fn blocks(sources: &[&str]) -> Vec<Json> {
    sources.iter().map(|source| event(source)).collect()
}

#[test]
fn empty_tool_call_args_predicate_covers_both_sides() {
    let mut decoder = state("bedrock/model", false, &[FIRST]);
    decoder.content_blocks = blocks(&[]);
    assert!(!decoder.check_empty_tool_call_args().unwrap());
    decoder.content_blocks = blocks(&[r#"{"text": "x"}"#, r#"{"toolUse": {"input": ""}}"#]);
    assert!(!decoder.check_empty_tool_call_args().unwrap());
    decoder.content_blocks = blocks(&[r#"{"toolUse": {"input": ""}}"#]);
    assert!(decoder.check_empty_tool_call_args().unwrap());
    decoder.content_blocks = blocks(&[r#"{"toolUse": {"input": " "}}"#]);
    assert!(!decoder.check_empty_tool_call_args().unwrap());
    decoder.content_blocks = blocks(&[
        r#"{"toolUse": {"input": ""}}"#,
        r#"{"toolUse": {"input": ""}}"#,
    ]);
    assert!(decoder.check_empty_tool_call_args().unwrap());
    decoder.content_blocks = blocks(&[r#"{"toolUse": {"input": "abc"}}"#]);
    assert!(!decoder.check_empty_tool_call_args().unwrap());
}

#[rstest]
#[case::false_flag("false")]
#[case::zero_number("0")]
#[case::empty_text(r#""""#)]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
#[case::null_value("null")]
fn trace_is_omitted_when_empty(#[case] trace: &str) {
    let mut decoder = state("bedrock/model", false, &[FIRST]);
    let source =
        format!(r#"{{"usage": {{"inputTokens": 1, "outputTokens": 2}}, "trace": {trace}}}"#);
    let chunk = decoder.converse_chunk_parser(&event(&source)).unwrap();
    assert!(chunk.provider_specific_fields.is_empty());
    assert_eq!(chunk.delta.content, None);
    assert_eq!(chunk.delta.role, None);
}

#[test]
fn usage_branch_never_estimates_reasoning_tokens() {
    assert_eq!(
        UnusedCounter.count_reasoning_tokens("some text").unwrap(),
        0
    );
}
