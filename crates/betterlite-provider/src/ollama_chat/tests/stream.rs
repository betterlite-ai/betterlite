use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::function::is_function_call_complete;
use super::super::ids::RandomIds;
use super::super::stream::chunk::{
    DeltaFunction, DeltaToolCall, OllamaStreamChunk, StreamChoice, StreamDelta,
};
use super::super::stream::{OllamaChatIterator, OllamaStreamStep};
use super::super::usage::Usage;
use super::support::SequenceIds;

const NOW: i64 = 1_700_000_000;

fn parsed(line: &str) -> OllamaStreamChunk {
    let mut iterator = OllamaChatIterator::default();
    iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap()
}

fn parsed_in(iterator: &mut OllamaChatIterator, line: &str) -> OllamaStreamChunk {
    iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap()
}

fn delta(line: &str) -> StreamDelta {
    parsed(line).choices.first().unwrap().delta.clone()
}

fn expected_delta(content: Option<&str>, reasoning: Option<&str>) -> StreamDelta {
    StreamDelta {
        content: content.map(str::to_owned),
        reasoning_content: reasoning.map(str::to_owned),
        tool_calls: None,
    }
}

#[test]
fn a_content_chunk_carries_the_delta_and_no_usage() {
    let chunk = parsed(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "Hello"}, "done": false}"#,
    );
    assert_eq!(chunk.id, "id-0");
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.created, NOW);
    assert_eq!(chunk.model, "llama3");
    assert_eq!(chunk.usage, None);
    assert_eq!(
        chunk.choices,
        vec![StreamChoice {
            index: 0,
            delta: expected_delta(Some("Hello"), None),
            finish_reason: None,
        }]
    );
}

#[test]
fn an_empty_content_gives_no_content_delta() {
    assert_eq!(
        delta(r#"{"model": "m", "message": {"content": ""}, "done": false}"#),
        StreamDelta::default()
    );
}

#[test]
fn a_missing_content_gives_no_content_delta() {
    assert_eq!(
        delta(r#"{"model": "m", "message": {"role": "assistant"}, "done": false}"#),
        StreamDelta::default()
    );
}

#[test]
fn reasoning_content_ends_when_the_answer_starts() {
    let mut iterator = OllamaChatIterator::default();
    let thinking = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"role": "assistant", "thinking": "why"}, "done": false}"#,
    );
    assert_eq!(
        thinking.choices.first().unwrap().delta,
        expected_delta(None, Some("why"))
    );
    let answer = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"role": "assistant", "content": "ans"}, "done": false}"#,
    );
    assert_eq!(
        answer.choices.first().unwrap().delta,
        expected_delta(Some("ans"), None)
    );
}

#[test]
fn think_tags_split_reasoning_from_content() {
    let mut iterator = OllamaChatIterator::default();
    let opening = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"role": "assistant", "content": "<think>in"}, "done": false}"#,
    );
    assert_eq!(
        opening.choices.first().unwrap().delta,
        expected_delta(None, Some("in"))
    );
    let closing = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"role": "assistant", "content": "side</think>out"}, "done": false}"#,
    );
    assert_eq!(
        closing.choices.first().unwrap().delta,
        expected_delta(Some("sideout"), None)
    );
}

#[test]
fn a_complete_tool_call_gets_an_id_and_json_arguments() {
    let chunk = parsed(
        r#"{"model": "m", "message": {"content": "x", "tool_calls": [{"function": {"name": "f", "arguments": {"a": 1}}}]}, "done": false}"#,
    );
    assert_eq!(
        chunk.choices.first().unwrap().delta.tool_calls,
        Some(vec![DeltaToolCall {
            id: Some(String::from("id-0")),
            kind: String::from("function"),
            index: 0,
            function: DeltaFunction {
                name: Some(String::from("f")),
                arguments: String::from(r#"{"a":1}"#),
            },
        }])
    );
}

#[test]
fn a_given_tool_call_index_is_kept_and_missing_ones_count_up_from_zero() {
    let chunk = parsed(
        r#"{"model": "m", "message": {"content": "x", "tool_calls": [{"index": 5, "function": {"name": "f"}}, {"function": {"name": "g"}}, {"function": {"name": "h"}}]}, "done": false}"#,
    );
    let indexes: Vec<u32> = chunk
        .choices
        .first()
        .unwrap()
        .delta
        .tool_calls
        .as_ref()
        .unwrap()
        .iter()
        .map(|call| call.index)
        .collect();
    assert_eq!(indexes, [5, 0, 1]);
}

#[test]
fn a_null_tool_call_type_is_a_function() {
    let chunk = parsed(
        r#"{"model": "m", "message": {"content": "x", "tool_calls": [{"type": null, "function": {"name": "f", "arguments": {"a": 1}}}]}, "done": false}"#,
    );
    let call = chunk
        .choices
        .first()
        .unwrap()
        .delta
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.kind, "function");
}

#[rstest]
#[case::empty_object("{}", false)]
#[case::incomplete_string(r#""{\"a\":""#, false)]
#[case::complete_string(r#""{}""#, true)]
#[case::empty_string(r#""""#, false)]
fn tool_call_ids_depend_on_complete_arguments(#[case] arguments: &str, #[case] has_id: bool) {
    let line = format!(
        r#"{{"model": "m", "message": {{"content": "x", "tool_calls": [{{"function": {{"name": "f", "arguments": {arguments}}}}}]}}, "done": false}}"#
    );
    let chunk = parsed(&line);
    let call = chunk
        .choices
        .first()
        .unwrap()
        .delta
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.id.is_some(), has_id);
}

#[test]
fn an_empty_tool_call_list_is_still_a_delta_and_finishes_with_tool_calls() {
    let mut iterator = OllamaChatIterator::default();
    let calls = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"content": "x", "tool_calls": []}, "done": false}"#,
    );
    assert_eq!(
        calls.choices.first().unwrap().delta.tool_calls,
        Some(Vec::new())
    );
    let done = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"content": "z"}, "done": true, "done_reason": "stop"}"#,
    );
    assert_eq!(
        done.choices.first().unwrap().finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[test]
fn a_null_tool_call_list_does_not_mark_the_stream_as_tool_calls() {
    let mut iterator = OllamaChatIterator::default();
    parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"content": "x", "tool_calls": null}, "done": false}"#,
    );
    let done = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"content": "z"}, "done": true}"#,
    );
    assert_eq!(
        done.choices.first().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(done.choices.first().unwrap().delta.tool_calls, None);
}

#[test]
fn a_done_chunk_carries_usage() {
    let chunk = parsed(
        r#"{"model": "m", "message": {"content": "x"}, "done": true, "done_reason": "stop", "prompt_eval_count": 3, "eval_count": 4}"#,
    );
    assert_eq!(
        chunk.usage,
        Some(Usage {
            prompt_tokens: 3,
            completion_tokens: 4,
            total_tokens: 7,
        })
    );
    assert_eq!(
        chunk.choices.first().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
}

#[rstest]
#[case::not_done(
    r#"{"model": "m", "message": {"content": "x"}, "done": false, "prompt_eval_count": 3, "eval_count": 4}"#
)]
#[case::missing_eval_count(
    r#"{"model": "m", "message": {"content": "x"}, "done": true, "prompt_eval_count": 3}"#
)]
#[case::null_counts(
    r#"{"model": "m", "message": {"content": "x"}, "done": true, "prompt_eval_count": null, "eval_count": null}"#
)]
fn chunks_without_both_counts_have_no_usage(#[case] line: &str) {
    assert_eq!(parsed(line).usage, None);
}

#[test]
fn random_ids_are_distinct_uuids() {
    let first = super::super::ids::IdSource::next_id(&RandomIds);
    uuid::Uuid::parse_str(&first).unwrap();
    assert_ne!(first, super::super::ids::IdSource::next_id(&RandomIds));
}

#[rstest]
#[case::length(r#""done_reason": "length","#, "length")]
#[case::stop(r#""done_reason": "stop","#, "stop")]
#[case::unmapped_load(r#""done_reason": "load","#, "stop")]
#[case::empty(r#""done_reason": "","#, "stop")]
#[case::null(r#""done_reason": null,"#, "stop")]
#[case::missing("", "stop")]
fn the_done_reason_is_mapped_or_defaults_to_stop(#[case] reason: &str, #[case] expected: &str) {
    let line = format!(r#"{{"model": "m", "message": {{"content": "x"}}, {reason} "done": true}}"#);
    assert_eq!(
        parsed(&line)
            .choices
            .first()
            .unwrap()
            .finish_reason
            .as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::missing_message(r#"{"model": "m", "done": false}"#, "message")]
#[case::missing_done(r#"{"model": "m", "message": {"content": "x"}}"#, "done")]
#[case::missing_model(r#"{"message": {"content": "x"}, "done": false}"#, "model")]
fn a_missing_chunk_field_is_a_400(#[case] line: &str, #[case] field: &str) {
    let mut iterator = OllamaChatIterator::default();
    let error = iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), Some(400));
    assert!(error.to_string().contains(field));
}

#[rstest]
#[case::not_an_object("[]")]
#[case::message_wrong_type(r#"{"model": "m", "message": 5, "done": false}"#)]
#[case::done_wrong_type(r#"{"model": "m", "message": {"content": "x"}, "done": "true"}"#)]
#[case::model_wrong_type(r#"{"model": 5, "message": {"content": "x"}, "done": false}"#)]
#[case::arguments_wrong_type(
    r#"{"model": "m", "message": {"content": "x", "tool_calls": [{"function": {"name": "f", "arguments": 5}}]}, "done": false}"#
)]
fn a_chunk_of_the_wrong_type_has_no_status(#[case] line: &str) {
    let mut iterator = OllamaChatIterator::default();
    let error = iterator
        .chunk_parser(line.as_bytes(), NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::data_prefix(r#"data: {"model": "m", "message": {"content": "x"}, "done": false}"#)]
#[case::no_prefix(r#"{"model": "m", "message": {"content": "x"}, "done": false}"#)]
fn next_chunk_accepts_a_data_prefix(#[case] line: &str) {
    let mut iterator = OllamaChatIterator::default();
    let step = iterator.next_chunk(line, NOW, &SequenceIds::new()).unwrap();
    assert!(matches!(step, OllamaStreamStep::Chunk(_)));
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::not_json("not json")]
#[case::null("null")]
#[case::zero("0")]
#[case::json_false("false")]
#[case::empty_array("[]")]
#[case::empty_object("{}")]
#[case::data_only("data: ")]
fn next_chunk_skips_lines_litellm_skips(#[case] line: &str) {
    let mut iterator = OllamaChatIterator::default();
    let step = iterator.next_chunk(line, NOW, &SequenceIds::new()).unwrap();
    assert!(matches!(step, OllamaStreamStep::Skipped));
}

#[rstest]
#[case::done_marker("[DONE]")]
#[case::data_done("data: [DONE]")]
#[case::done_inside_a_string(
    r#"{"model": "m", "message": {"content": "a[DONE]b"}, "done": false}"#
)]
fn a_done_marker_ends_the_iterator(#[case] line: &str) {
    let mut iterator = OllamaChatIterator::default();
    let step = iterator.next_chunk(line, NOW, &SequenceIds::new()).unwrap();
    assert!(matches!(step, OllamaStreamStep::Done));
}

#[test]
fn a_truthy_non_object_line_is_an_error() {
    let mut iterator = OllamaChatIterator::default();
    let error = iterator
        .next_chunk("[1, 2]", NOW, &SequenceIds::new())
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[test]
fn a_skipped_line_does_not_reset_the_reasoning_state() {
    let mut iterator = OllamaChatIterator::default();
    parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"thinking": "why"}, "done": false}"#,
    );
    let _skipped = iterator
        .next_chunk("not json", NOW, &SequenceIds::new())
        .unwrap();
    let closing = parsed_in(
        &mut iterator,
        r#"{"model": "m", "message": {"content": "</think>done"}, "done": false}"#,
    );
    assert_eq!(
        closing.choices.first().unwrap().delta,
        expected_delta(Some("done"), None)
    );
}

#[rstest]
#[case::object(Json::Object(IndexMap::new()), true)]
#[case::text(Json::Text(String::from(r#"{"a": 1}"#)), true)]
#[case::incomplete(Json::Text(String::from(r#"{"a":"#)), false)]
#[case::empty_text(Json::Text(String::new()), false)]
#[case::null(Json::Null, false)]
#[case::number(Json::Number(serde_json::Number::from(5_i64)), false)]
fn function_call_completeness_matches_litellm(#[case] arguments: Json, #[case] expected: bool) {
    assert_eq!(is_function_call_complete(&arguments), expected);
}
