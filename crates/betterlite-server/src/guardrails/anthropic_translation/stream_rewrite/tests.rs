#![cfg(test)]

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::iter;
use std::path::PathBuf;
use std::str;

use super::ended::{rewrite_ended_stream_text, rewrite_ended_tool_calls};
use super::events::{rewrite_ended_stream_events, rewritten_event};
use super::json_dumps::dumped_json;
use super::line::{rewrite_sse_events, rewrite_sse_line};
use super::rewriter::EventRewriter;
use crate::guardrails::anthropic_translation::sse_events::SseChunk;
use crate::guardrails::tool_call_identity::ToolCallShape;
use rstest::rstest;
use serde_json::{Map, Value};

const TEXT_DELTA: &str = "data: {\"delta\": {\"type\": \"text_delta\", \"text\": \"old\"}, \"index\": 0, \"type\": \"content_block_delta\"}";

const TOOL_STREAM: &str = "data: {\"content_block\": {\"name\": \"lookup\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{\\\"q\\\": 1}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"partial_json\": \"\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"content_block\": {\"name\": \"keep\", \"type\": \"tool_use\"}, \"index\": 1, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}";

const TOOL_STREAM_REWRITTEN: &str = "data: {\"content_block\": {\"name\": \"search\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{\\\"q\\\": 2}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"partial_json\": \"\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"content_block\": {\"name\": \"keep\", \"type\": \"tool_use\"}, \"index\": 1, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}";

fn bytes(text: &str) -> SseChunk {
    SseChunk::Bytes(text.as_bytes().to_vec())
}

fn event(text: &str) -> SseChunk {
    SseChunk::Event(
        serde_json::from_str::<Value>(text)
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    )
}

fn text_of(chunk: &SseChunk) -> String {
    match chunk {
        SseChunk::Bytes(found) => str::from_utf8(found).unwrap().to_owned(),
        SseChunk::Event(_) | SseChunk::Other => panic!("a bytes chunk"),
    }
}

fn shape(name: Option<&str>, arguments: &str) -> ToolCallShape {
    ToolCallShape {
        name: name.map(str::to_owned),
        arguments: arguments.to_owned(),
    }
}

fn text_rewriter(text: &str) -> EventRewriter {
    EventRewriter::TextDelta {
        replacements: VecDeque::from([text.to_owned()]),
    }
}

fn line_counts(text: &str) -> Vec<usize> {
    text.split("\n\n")
        .map(|block| block.split('\n').count())
        .collect()
}

#[test]
fn text_rewrite_puts_the_text_in_the_first_delta_and_blanks_the_rest() {
    let mut chunks = vec![bytes(&format!("{TEXT_DELTA}\n\n{TEXT_DELTA}"))];
    rewrite_ended_stream_text(&mut chunks, "new *text*", "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"delta\": {\"text\": \"new *text*\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"text\": \"\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn text_rewrite_leaves_other_events_and_framing_alone() {
    let source = "event: message_start\ndata: {\"message\": {\"id\": \"msg_1\"}, \"type\": \"message_start\"}\n\nevent: ping\ndata: {\"type\": \"ping\"}\n\ndata: {\"delta\": {\"type\": \"text_delta\", \"text\": \"old\"}, \"index\": 0, \"type\": \"content_block_delta\"}";
    let mut chunks = vec![bytes(source)];
    rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "event: message_start\ndata: {\"message\": {\"id\": \"msg_1\"}, \"type\": \"message_start\"}\n\nevent: ping\ndata: {\"type\": \"ping\"}\n\ndata: {\"delta\": {\"text\": \"new\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn text_rewrite_rewrites_a_dict_event_and_keeps_a_non_delta_one() {
    let untouched = event("{\"message\": {\"id\": \"msg_1\"}, \"type\": \"message_start\"}");
    let mut chunks = vec![
        untouched.clone(),
        event(
            "{\"delta\": {\"type\": \"text_delta\", \"text\": \"old\"}, \"index\": 0, \"type\": \"content_block_delta\"}",
        ),
    ];
    rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap();
    assert_eq!(chunks[0], untouched);
    assert_eq!(
        chunks[1],
        event(
            "{\"delta\": {\"text\": \"new\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}"
        )
    );
}

#[test]
fn text_rewrite_leaves_an_event_without_the_section_it_rewrites() {
    let missing: Value =
        serde_json::from_str("{\"index\": 0, \"type\": \"content_block_start\"}").unwrap();
    let mut rewriter = EventRewriter::ToolUse {
        rewrites_by_block: iter::once((0_i64, shape(Some("search"), "{}"))).collect(),
        arguments_sent: BTreeSet::new(),
    };
    assert_eq!(
        rewritten_event(missing.as_object().unwrap(), &mut rewriter),
        None
    );
}

#[test]
fn text_rewrite_reports_an_undeliverable_rewrite_without_a_text_delta() {
    let mut chunks = vec![event("{\"type\": \"content_block_start\"}")];
    let failure = rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap_err();
    assert_eq!(failure.guardrail_name, "guard");
    assert_eq!(
        failure.reason,
        "the buffered stream carries no text_delta event to land the text rewrite on"
    );
}

#[test]
fn text_rewrite_keeps_an_indented_data_line_unrewritten() {
    let indented = " data: {\"delta\": {\"type\": \"text_delta\", \"text\": \"old\"}, \"index\": 0, \"type\": \"content_block_delta\"}";
    let mut chunks = vec![bytes(indented)];
    rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap();
    assert_eq!(text_of(&chunks[0]), indented);
}

#[test]
fn tool_rewrite_lands_the_nth_call_on_the_nth_block() {
    let mut chunks = vec![bytes(TOOL_STREAM)];
    let pre = [
        shape(Some("lookup"), "{\"q\": 1}"),
        shape(Some("keep"), "{}"),
    ];
    let post = [
        shape(Some("search"), "{\"q\": 2}"),
        shape(Some("keep"), "{}"),
    ];
    rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap();
    assert_eq!(text_of(&chunks[0]), TOOL_STREAM_REWRITTEN);
}

#[test]
fn tool_rewrite_returns_at_once_when_the_shapes_are_equal() {
    let mut chunks = vec![bytes(TOOL_STREAM)];
    let same = [shape(Some("lookup"), "{\"q\": 1}")];
    rewrite_ended_tool_calls(&mut chunks, &same, &same, "guard").unwrap();
    assert_eq!(text_of(&chunks[0]), TOOL_STREAM);
}

#[test]
fn tool_rewrite_reports_a_block_count_mismatch() {
    let mut chunks = vec![bytes(TOOL_STREAM)];
    let pre = [
        shape(Some("lookup"), "{\"q\": 1}"),
        shape(Some("keep"), "{}"),
    ];
    let post = [
        shape(Some("a"), "{}"),
        shape(Some("b"), "{}"),
        shape(Some("c"), "{}"),
    ];
    let failure = rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap_err();
    assert_eq!(
        failure.reason,
        "the guardrail returned 3 tool calls for a stream that carried 2 tool_use blocks"
    );
}

#[test]
fn tool_rewrite_keeps_the_name_when_the_rewritten_name_is_none() {
    let mut chunks = vec![bytes(TOOL_STREAM)];
    let pre = [
        shape(Some("lookup"), "{\"q\": 1}"),
        shape(Some("keep"), "{}"),
    ];
    let post = [shape(None, "{\"q\": 2}"), shape(Some("keep"), "{}")];
    rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"content_block\": {\"name\": \"lookup\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{\\\"q\\\": 2}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"partial_json\": \"\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"content_block\": {\"name\": \"keep\", \"type\": \"tool_use\"}, \"index\": 1, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn tool_rewrite_ignores_a_text_content_block_start() {
    let stream = "data: {\"content_block\": {\"name\": \"lookup\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"content_block\": {\"text\": \"hi\", \"type\": \"text\"}, \"index\": 1, \"type\": \"content_block_start\"}";
    let mut chunks = vec![bytes(stream)];
    let pre = [shape(Some("lookup"), "{}")];
    let post = [shape(Some("search"), "{}")];
    rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"content_block\": {\"name\": \"search\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"content_block\": {\"text\": \"hi\", \"type\": \"text\"}, \"index\": 1, \"type\": \"content_block_start\"}"
    );
}

#[test]
fn tool_rewrite_leaves_an_unchanged_block_fragmented() {
    let stream = "data: {\"content_block\": {\"name\": \"lookup\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"content_block\": {\"name\": \"keep\", \"type\": \"tool_use\"}, \"index\": 1, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{\\\"x\\\":\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"partial_json\": \"3}\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}";
    let mut chunks = vec![bytes(stream)];
    let pre = [
        shape(Some("lookup"), "{}"),
        shape(Some("keep"), "{\"x\": 3}"),
    ];
    let post = [
        shape(Some("search"), "{}"),
        shape(Some("keep"), "{\"x\": 3}"),
    ];
    rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"content_block\": {\"name\": \"search\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}\n\ndata: {\"content_block\": {\"name\": \"keep\", \"type\": \"tool_use\"}, \"index\": 1, \"type\": \"content_block_start\"}\n\ndata: {\"delta\": {\"partial_json\": \"{\\\"x\\\":\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"partial_json\": \"3}\", \"type\": \"input_json_delta\"}, \"index\": 1, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn text_rewrite_ignores_events_that_only_look_like_text_deltas() {
    let stream = "data: {\"delta\": {\"text\": \"x\", \"type\": \"text_delta\"}, \"type\": \"content_block_stop\"}\n\ndata: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"text\": \"old\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}";
    let mut chunks = vec![bytes(stream)];
    rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"delta\": {\"text\": \"x\", \"type\": \"text_delta\"}, \"type\": \"content_block_stop\"}\n\ndata: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"input_json_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"delta\": {\"text\": \"new\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn tool_rewrite_ignores_an_event_of_another_type_for_the_same_index() {
    let mut chunks = vec![bytes(
        "data: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"content_block\": {\"name\": \"lookup\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}",
    )];
    let pre = [shape(Some("lookup"), "{\"q\": 1}")];
    let post = [shape(Some("search"), "{\"q\": 2}")];
    rewrite_ended_tool_calls(&mut chunks, &pre, &post, "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"delta\": {\"partial_json\": \"{}\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}\n\ndata: {\"content_block\": {\"name\": \"search\", \"type\": \"tool_use\"}, \"index\": 0, \"type\": \"content_block_start\"}"
    );
}

#[test]
fn bytes_rewrite_returns_invalid_utf8_unchanged() {
    let invalid = b"data: {\"text\": \"\xff\"}\n\n".to_vec();
    assert_eq!(
        rewrite_sse_events(&invalid, &mut text_rewriter("new")),
        invalid
    );
}

#[test]
fn bytes_rewrite_keeps_lines_it_cannot_read() {
    let source = b"event: message_start\ndata: not json\ndata: [1]\n\n".to_vec();
    assert_eq!(
        rewrite_sse_events(&source, &mut text_rewriter("new")),
        source
    );
    assert_eq!(
        rewrite_sse_line("event: ping", &mut text_rewriter("new")),
        "event: ping"
    );
    assert_eq!(
        rewrite_sse_line(" data: {\"type\": \"ping\"}", &mut text_rewriter("new")),
        " data: {\"type\": \"ping\"}"
    );
}

#[test]
fn bytes_rewrite_writes_a_rewritten_line_as_data_space_and_json_dumps() {
    let mut chunks = vec![bytes(
        "data:{\"delta\":{\"type\":\"text_delta\",\"text\":\"old\"},\"index\":0,\"type\":\"content_block_delta\"}",
    )];
    rewrite_ended_stream_text(&mut chunks, "new", "guard").unwrap();
    assert_eq!(
        text_of(&chunks[0]),
        "data: {\"delta\": {\"text\": \"new\", \"type\": \"text_delta\"}, \"index\": 0, \"type\": \"content_block_delta\"}"
    );
}

#[test]
fn bytes_rewrite_keeps_the_separators_of_the_input() {
    let source = "data: {\"type\": \"ping\"}\n\n\n\ndata: {\"type\": \"ping\"}";
    let mut chunks = vec![bytes(source)];
    rewrite_ended_stream_events(&mut chunks, &mut text_rewriter("new"));
    assert_eq!(text_of(&chunks[0]), source);
}

#[test]
fn bytes_rewrite_keeps_an_other_chunk() {
    let mut chunks = vec![SseChunk::Other];
    rewrite_ended_stream_events(&mut chunks, &mut text_rewriter("new"));
    assert_eq!(chunks, vec![SseChunk::Other]);
}

#[test]
fn json_dumps_escapes_non_ascii() {
    let value: Map<String, Value> =
        serde_json::from_str("{\"text\": \"h\u{e9}llo \u{1f600}\"}").unwrap();
    assert_eq!(
        dumped_json(&value).unwrap(),
        "{\"text\": \"h\\u00e9llo \\ud83d\\ude00\"}"
    );
}

#[test]
fn json_dumps_escapes_the_delete_character_and_keeps_printable_ascii() {
    let value: Map<String, Value> =
        serde_json::from_str("{\"text\": \"a\u{7f} ~\\\"\\\\\"}").unwrap();
    assert_eq!(
        dumped_json(&value).unwrap(),
        "{\"text\": \"a\\u007f ~\\\"\\\\\"}"
    );
}

#[test]
fn json_dumps_writes_the_default_separators() {
    let value: Map<String, Value> =
        serde_json::from_str("{\"b\": {\"c\": null}, \"a\": [1, 2]}").unwrap();
    assert_eq!(
        dumped_json(&value).unwrap(),
        "{\"a\": [1, 2], \"b\": {\"c\": null}}"
    );
}

#[rstest]
fn replays_the_anthropic_stream_rewrite_corpus(
    #[files("fuzz/corpus/anthropic_stream_rewrite/*")] path: PathBuf,
) {
    let source = fs::read(path).unwrap();
    let rewritten = rewrite_sse_events(&source, &mut text_rewriter("replacement"));
    let Ok(decoded) = str::from_utf8(&source) else {
        assert_eq!(rewritten, source);
        return;
    };
    let output = str::from_utf8(&rewritten).unwrap();
    assert_eq!(line_counts(decoded), line_counts(output));
    let mut tools = EventRewriter::ToolUse {
        rewrites_by_block: iter::once((0_i64, shape(Some("name"), "{}"))).collect(),
        arguments_sent: BTreeSet::new(),
    };
    let tool_output = rewrite_sse_events(&source, &mut tools);
    assert_eq!(
        line_counts(decoded),
        line_counts(str::from_utf8(&tool_output).unwrap())
    );
}
