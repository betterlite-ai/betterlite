#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, JsonObject, JsonValue};
use rstest::rstest;
use uuid::Uuid;

use super::{StreamError, StreamEvent, chunk_parser};
use crate::gigachat_utils::IdSource;

const FIXED: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;

struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(FIXED)
    }
}

fn parse(event: &[u8]) -> StreamEvent {
    chunk_parser(event, &FixedIds).unwrap()
}

fn chunk(event: &str) -> GenericChunk {
    match parse(event.as_bytes()) {
        StreamEvent::Chunk(chunk) => *chunk,
        StreamEvent::Done => panic!("the event ended the stream"),
    }
}

fn tool_use(name: &str, arguments: &str) -> JsonValue {
    let mut function = JsonObject::new();
    function.insert(String::from("name"), JsonValue::String(name.to_owned()));
    function.insert(
        String::from("arguments"),
        JsonValue::String(arguments.to_owned()),
    );
    let mut tool = JsonObject::new();
    tool.insert(
        String::from("id"),
        JsonValue::String(String::from("call_0123456789ab4def81234567")),
    );
    tool.insert(
        String::from("type"),
        JsonValue::String(String::from("function")),
    );
    tool.insert(String::from("function"), JsonValue::Object(function));
    tool.insert(
        String::from("index"),
        JsonValue::Number(serde_json::Number::from(0_i64)),
    );
    JsonValue::Object(tool)
}

fn empty_chunk() -> GenericChunk {
    GenericChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index: Some(0),
        provider_specific_fields: None,
    }
}

#[test]
fn a_content_delta_is_kept() {
    let parsed = chunk(r#"{"choices": [{"index": 0, "delta": {"content": "hello"}}]}"#);
    assert_eq!(parsed.text, "hello");
    assert!(!parsed.is_finished);
    assert_eq!(parsed.finish_reason, "");
    assert_eq!(parsed.index, Some(0));
}

#[test]
fn a_null_content_reads_as_empty_text() {
    let parsed = chunk(r#"{"choices": [{"delta": {"content": null}}]}"#);
    assert_eq!(parsed.text, "");
}

#[test]
fn a_null_delta_reads_as_an_empty_delta() {
    let parsed = chunk(r#"{"choices": [{"delta": null}]}"#);
    assert_eq!(parsed.text, "");
    assert_eq!(parsed.tool_use, None);
}

#[test]
fn a_finish_reason_finishes_the_chunk() {
    let parsed = chunk(r#"{"choices": [{"delta": {}, "finish_reason": "stop"}]}"#);
    assert!(parsed.is_finished);
    assert_eq!(parsed.finish_reason, "stop");
}

#[test]
fn a_chunk_without_choices_is_empty_even_with_usage() {
    let parsed =
        chunk(r#"{"usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}}"#);
    assert_eq!(parsed, empty_chunk());
}

#[rstest]
#[case::missing_choices("{}")]
#[case::empty_choices(r#"{"choices": []}"#)]
#[case::not_json("not json")]
#[case::empty("")]
fn an_empty_chunk(#[case] event: &str) {
    assert_eq!(chunk(event), empty_chunk());
}

#[test]
fn the_data_prefix_is_removed_once() {
    let parsed = chunk(r#"data: {"choices": [{"delta": {"content": "a"}}]}"#);
    assert_eq!(parsed.text, "a");
    assert_eq!(
        chunk(r#"data: data: {"choices": [{"delta": {"content": "a"}}]}"#),
        empty_chunk()
    );
}

#[rstest]
#[case::plain("[DONE]")]
#[case::data_prefixed("data: [DONE]")]
#[case::padded("  [DONE]  ")]
fn done_ends_the_stream(#[case] event: &str) {
    assert_eq!(parse(event.as_bytes()), StreamEvent::Done);
}

#[test]
fn invalid_utf8_is_an_empty_chunk() {
    assert_eq!(
        chunk_parser(&[0xff, 0xfe], &FixedIds).unwrap(),
        StreamEvent::Chunk(Box::new(empty_chunk()))
    );
}

#[rstest]
#[case::choices_not_a_list(r#"{"choices": "nope"}"#)]
#[case::usage_not_an_object(r#"{"choices": [], "usage": 5}"#)]
#[case::scalar_event("5")]
#[case::array_event("[]")]
fn a_json_event_of_the_wrong_shape_is_an_error(#[case] event: &str) {
    assert!(matches!(
        chunk_parser(event.as_bytes(), &FixedIds),
        Err(StreamError::Chunk { .. })
    ));
}

#[test]
fn a_function_call_delta_becomes_tool_use() {
    let parsed = chunk(
        r#"{"choices": [{"index": 2, "delta": {"function_call": {"name": "lookup", "arguments": {"city": "prague"}}}, "finish_reason": "function_call"}]}"#,
    );
    assert_eq!(
        parsed.tool_use,
        Some(tool_use("lookup", r#"{"city":"prague"}"#))
    );
    assert_eq!(parsed.finish_reason, "tool_calls");
    assert!(parsed.is_finished);
    assert_eq!(parsed.index, Some(2));
}

#[test]
fn a_function_call_without_arguments_writes_an_empty_object() {
    let parsed = chunk(
        r#"{"choices": [{"delta": {"function_call": {"name": "lookup"}}, "finish_reason": "function_call"}]}"#,
    );
    assert_eq!(parsed.tool_use, Some(tool_use("lookup", "{}")));
}

#[test]
fn a_function_call_without_a_name_uses_an_empty_name() {
    let parsed = chunk(
        r#"{"choices": [{"delta": {"function_call": {"name": 5, "arguments": "x"}}, "finish_reason": "function_call"}]}"#,
    );
    assert_eq!(parsed.tool_use, Some(tool_use("", "x")));
}

#[test]
fn an_empty_function_call_leaves_the_finish_reason_alone() {
    let parsed = chunk(
        r#"{"choices": [{"delta": {"function_call": {}}, "finish_reason": "function_call"}]}"#,
    );
    assert_eq!(parsed.tool_use, None);
    assert_eq!(parsed.finish_reason, "function_call");
    assert!(parsed.is_finished);
}

#[test]
fn a_function_call_on_another_finish_reason_stays_a_plain_delta() {
    let parsed = chunk(
        r#"{"choices": [{"delta": {"function_call": {"name": "lookup"}}, "finish_reason": "stop"}]}"#,
    );
    assert_eq!(parsed.tool_use, None);
    assert_eq!(parsed.finish_reason, "stop");
}

#[test]
fn usage_goes_through_the_shared_conversion() {
    let parsed = chunk(
        r#"{"choices": [{"delta": {}}], "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12, "precached_prompt_tokens": 5}}"#,
    );
    let usage = parsed.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 15);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 17);
    let details = usage.prompt_tokens_details.unwrap();
    assert_eq!(details.cached_tokens, Some(5));
    assert_eq!(usage.completion_tokens_details, None);
}

#[test]
fn an_empty_usage_object_adds_no_usage() {
    let parsed = chunk(r#"{"choices": [{"delta": {}}], "usage": {}}"#);
    assert_eq!(parsed.usage, None);
}

#[test]
fn an_unknown_usage_key_still_marks_usage_present() {
    let parsed = chunk(r#"{"choices": [{"delta": {}}], "usage": {"counted_elsewhere": 1}}"#);
    let usage = parsed.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.prompt_tokens_details, None);
}

#[test]
fn a_usage_counter_too_large_for_the_chunk_usage_stays_at_the_largest_value() {
    let event = format!(
        r#"{{"choices": [{{"delta": {{}}}}], "usage": {{"prompt_tokens": {}}}}}"#,
        u64::MAX
    );
    let Ok(StreamEvent::Chunk(parsed)) = chunk_parser(event.as_bytes(), &FixedIds) else {
        panic!("the chunk should parse");
    };
    assert_eq!(parsed.usage.unwrap().prompt_tokens, i64::MAX);
}

#[test]
fn a_usage_sum_that_overflows_stays_at_the_largest_value() {
    let event = format!(
        r#"{{"choices": [{{"delta": {{}}}}], "usage": {{"prompt_tokens": {max}, "precached_prompt_tokens": 1}}}}"#,
        max = u64::MAX
    );
    let Ok(StreamEvent::Chunk(parsed)) = chunk_parser(event.as_bytes(), &FixedIds) else {
        panic!("the chunk should parse");
    };
    let usage = parsed.usage.unwrap();
    assert_eq!(usage.prompt_tokens, i64::MAX);
    assert_eq!(usage.prompt_tokens_details.unwrap().cached_tokens, Some(1));
}

#[rstest]
fn replays_the_gigachat_stream_chunk_corpus(
    #[files("fuzz/corpus/gigachat_stream_chunk/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _parsed = chunk_parser(&bytes, &FixedIds);
}
