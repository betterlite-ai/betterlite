#![cfg(test)]
#![expect(
    clippy::disallowed_methods,
    reason = "test fixtures are built with serde_json::json!, whose Into calls the lint table bans"
)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::stream::{self, StreamExt as _};
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    GenAiOutputError, GenAiStreamState, translate_chunk, translate_completion, translate_stream,
};

fn chat_response(body: Value) -> ChatResponse {
    serde_json::from_value(body).expect("a chat completion response")
}

fn completion(message: Value, finish_reason: Value) -> ChatResponse {
    chat_response(json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "m",
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8},
    }))
}

fn assistant(content: Value, tool_calls: Value) -> Value {
    json!({"role": "assistant", "content": content, "tool_calls": tool_calls})
}

fn translated(response: &ChatResponse) -> Value {
    serde_json::to_value(translate_completion(response).expect("the translated response"))
        .expect("the response serializes")
}

fn stream_chunk(delta: Value, finish_reason: Option<&str>, usage: Option<Value>) -> Chunk {
    let mut body = json!({
        "id": "chunk-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    });
    if let Some(found) = usage {
        body["usage"] = found;
    }
    serde_json::from_value(body).expect("a stream chunk")
}

fn tool_delta(index: u32, name: Option<&str>, arguments: &str) -> Value {
    json!({"tool_calls": [{"index": index, "function": {"name": name, "arguments": arguments}}]})
}

fn translated_chunk(chunk: &Chunk, state: &mut GenAiStreamState) -> Value {
    serde_json::to_value(translate_chunk(chunk, state).expect("a translated chunk"))
        .expect("the chunk serializes")
}

fn parts_of(value: &Value) -> Value {
    value["candidates"][0]["content"]["parts"].clone()
}

fn payload(frame: &str) -> Value {
    let json = frame
        .strip_prefix("data: ")
        .and_then(|rest| rest.strip_suffix("\n\n"))
        .expect("an SSE data frame");
    serde_json::from_str(json).expect("a JSON payload")
}

async fn framed(chunks: Vec<Chunk>) -> Vec<String> {
    translate_stream(stream::iter(chunks).boxed())
        .map(|frame| String::from_utf8(frame.expect("a framed chunk")).expect("utf-8"))
        .collect()
        .await
}

#[test]
fn a_completion_maps_the_first_choice_message_finish_reason_and_usage() {
    let response = chat_response(json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "m",
        "choices": [
            {"index": 0, "message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"},
            {"index": 1, "message": {"role": "assistant", "content": "second"}, "finish_reason": "length"},
        ],
        "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8},
    }));
    assert_eq!(
        translated(&response),
        json!({
            "candidates": [{
                "content": {"parts": [{"text": "hello"}], "role": "model"},
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": [],
            }],
            "usageMetadata": {
                "promptTokenCount": 3,
                "candidatesTokenCount": 5,
                "totalTokenCount": 8,
            },
        })
    );
}

#[test]
fn a_completion_without_choices_is_an_error() {
    let response = chat_response(json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "m",
        "choices": [],
    }));
    assert!(matches!(
        translate_completion(&response),
        Err(GenAiOutputError::NoChoices)
    ));
}

#[test]
fn a_message_without_text_or_calls_yields_one_empty_text_part() {
    let response = completion(assistant(json!(null), json!(null)), json!("stop"));
    assert_eq!(parts_of(&translated(&response)), json!([{"text": ""}]));
}

#[test]
fn message_text_and_tool_calls_become_text_and_function_call_parts() {
    let message = assistant(
        json!("calling"),
        json!([{
            "id": "call_1",
            "type": "function",
            "function": {"name": "lookup", "arguments": "{\"city\":\"Paris\"}"},
        }]),
    );
    let response = completion(message, json!("tool_calls"));
    assert_eq!(
        parts_of(&translated(&response)),
        json!([
            {"text": "calling"},
            {"functionCall": {"name": "lookup", "args": {"city": "Paris"}}},
        ])
    );
}

#[test]
fn empty_message_text_is_dropped_before_tool_calls() {
    let message = assistant(
        json!(""),
        json!([{
            "id": "call_1",
            "type": "function",
            "function": {"name": "lookup", "arguments": "{}"},
        }]),
    );
    let response = completion(message, json!("tool_calls"));
    assert_eq!(
        parts_of(&translated(&response)),
        json!([{"functionCall": {"name": "lookup", "args": {}}}])
    );
}

#[rstest]
#[case::empty(json!(""), json!({}))]
#[case::malformed(json!("{not json"), json!({}))]
#[case::object(json!("{\"city\":\"Paris\"}"), json!({"city": "Paris"}))]
#[case::scalar(json!("5"), json!(5))]
fn a_tool_call_maps_its_arguments(#[case] arguments: Value, #[case] expected: Value) {
    let message = assistant(
        json!(null),
        json!([{
            "id": "call_1",
            "type": "function",
            "function": {"name": "lookup", "arguments": arguments},
        }]),
    );
    let response = completion(message, json!("tool_calls"));
    assert_eq!(
        parts_of(&translated(&response)),
        json!([{"functionCall": {"name": "lookup", "args": expected}}])
    );
}

#[test]
fn a_tool_call_with_a_missing_name_becomes_undefined_tool_name() {
    let message = assistant(
        json!(null),
        json!([{
            "id": "call_1",
            "type": "function",
            "function": {"name": "", "arguments": "{}"},
        }]),
    );
    let response = completion(message, json!("tool_calls"));
    assert_eq!(
        parts_of(&translated(&response)),
        json!([{"functionCall": {"name": "undefined_tool_name", "args": {}}}])
    );
}

#[rstest]
#[case::stop("stop", "STOP")]
#[case::length("length", "MAX_TOKENS")]
#[case::content_filter("content_filter", "SAFETY")]
#[case::tool_calls("tool_calls", "STOP")]
#[case::function_call("function_call", "STOP")]
#[case::unmapped("weird", "STOP")]
#[case::empty("", "STOP")]
fn finish_reasons_map_to_google_genai_reasons(#[case] reason: &str, #[case] expected: &str) {
    let response = completion(assistant(json!("hi"), json!(null)), json!(reason));
    assert_eq!(
        translated(&response)["candidates"][0]["finishReason"],
        json!(expected)
    );
}

#[test]
fn an_absent_finish_reason_maps_to_stop() {
    let response = completion(assistant(json!("hi"), json!(null)), json!(null));
    assert_eq!(
        translated(&response)["candidates"][0]["finishReason"],
        json!("STOP")
    );
}

#[test]
fn a_text_delta_becomes_one_text_part() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(json!({"content": "Hello"}), None, None);
    assert_eq!(
        translated_chunk(&chunk, &mut state),
        json!({
            "candidates": [{
                "content": {"parts": [{"text": "Hello"}], "role": "model"},
                "finishReason": null,
                "index": 0,
                "safetyRatings": [],
            }],
        })
    );
}

#[test]
fn an_empty_delta_produces_no_chunk() {
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&stream_chunk(json!({}), None, None), &mut state).is_none());
}

#[test]
fn an_empty_delta_with_a_finish_reason_produces_a_chunk() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(json!({}), Some("stop"), None);
    assert_eq!(
        translated_chunk(&chunk, &mut state),
        json!({
            "candidates": [{
                "content": {"parts": [], "role": "model"},
                "finishReason": "STOP",
                "index": 0,
                "safetyRatings": [],
            }],
            "usageMetadata": {
                "promptTokenCount": 0,
                "candidatesTokenCount": 0,
                "totalTokenCount": 0,
            },
        })
    );
}

#[test]
fn an_empty_content_string_produces_no_part() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(json!({"content": ""}), None, None);
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn a_chunk_without_choices_produces_nothing() {
    let chunk: Chunk = serde_json::from_value(json!({
        "id": "chunk-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [],
    }))
    .expect("a stream chunk");
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn streaming_tool_calls_accumulate_by_index() {
    let mut state = GenAiStreamState::default();
    let first = stream_chunk(tool_delta(0, Some("lookup"), "{\"ci"), None, None);
    let second = stream_chunk(tool_delta(0, None, "ty\": \"Par"), None, None);
    let third = stream_chunk(tool_delta(0, None, "is\"}"), None, None);
    assert!(translate_chunk(&first, &mut state).is_none());
    assert!(translate_chunk(&second, &mut state).is_none());
    assert_eq!(
        parts_of(&translated_chunk(&third, &mut state)),
        json!([{"functionCall": {"name": "lookup", "args": {"city": "Paris"}}}])
    );
    assert!(state.accumulated_tool_calls.is_empty());
}

#[test]
fn a_tool_call_waits_for_its_name_before_it_emits() {
    let mut state = GenAiStreamState::default();
    let arguments = stream_chunk(tool_delta(0, None, "{\"a\":1}"), None, None);
    assert!(translate_chunk(&arguments, &mut state).is_none());
    let name = stream_chunk(tool_delta(0, Some("f"), ""), None, None);
    assert_eq!(
        parts_of(&translated_chunk(&name, &mut state)),
        json!([{"functionCall": {"name": "f", "args": {"a": 1}}}])
    );
}

#[test]
fn a_later_name_replaces_the_accumulated_one_and_an_empty_name_keeps_it() {
    let mut state = GenAiStreamState::default();
    let first_name = stream_chunk(tool_delta(0, Some("a"), "{\"x\":"), None, None);
    assert!(translate_chunk(&first_name, &mut state).is_none());
    let second_name = stream_chunk(tool_delta(0, Some("b"), ""), None, None);
    assert!(translate_chunk(&second_name, &mut state).is_none());
    let empty_name = stream_chunk(tool_delta(0, Some(""), "1}"), None, None);
    assert_eq!(
        parts_of(&translated_chunk(&empty_name, &mut state)),
        json!([{"functionCall": {"name": "b", "args": {"x": 1}}}])
    );
}

#[test]
fn two_tool_calls_accumulate_separately_by_index() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(
        json!({"tool_calls": [
            {"index": 0, "function": {"name": "a", "arguments": "{\"x\":1}"}},
            {"index": 1, "function": {"name": "b", "arguments": "{\"y\":2}"}},
        ]}),
        None,
        None,
    );
    assert_eq!(
        parts_of(&translated_chunk(&chunk, &mut state)),
        json!([
            {"functionCall": {"name": "a", "args": {"x": 1}}},
            {"functionCall": {"name": "b", "args": {"y": 2}}},
        ])
    );
}

#[test]
fn malformed_arguments_wait_for_a_later_chunk() {
    let mut state = GenAiStreamState::default();
    let broken = stream_chunk(tool_delta(0, Some("f"), "{\"a\":"), None, None);
    assert!(translate_chunk(&broken, &mut state).is_none());
    let fixed = stream_chunk(tool_delta(0, None, "1}"), None, None);
    assert_eq!(
        parts_of(&translated_chunk(&fixed, &mut state)),
        json!([{"functionCall": {"name": "f", "args": {"a": 1}}}])
    );
}

#[test]
fn arguments_that_never_parse_are_dropped_at_end_of_stream() {
    let mut state = GenAiStreamState::default();
    let broken = stream_chunk(tool_delta(0, Some("f"), "{\"a\":"), None, None);
    assert!(translate_chunk(&broken, &mut state).is_none());
    let finish = stream_chunk(json!({}), Some("stop"), None);
    assert_eq!(parts_of(&translated_chunk(&finish, &mut state)), json!([]));
}

#[test]
fn a_completed_tool_call_starts_fresh_at_the_same_index() {
    let mut state = GenAiStreamState::default();
    let first = stream_chunk(tool_delta(0, Some("a"), "{\"x\":1}"), None, None);
    assert_eq!(
        parts_of(&translated_chunk(&first, &mut state)),
        json!([{"functionCall": {"name": "a", "args": {"x": 1}}}])
    );
    let second = stream_chunk(tool_delta(0, Some("b"), "{\"y\":2}"), None, None);
    assert_eq!(
        parts_of(&translated_chunk(&second, &mut state)),
        json!([{"functionCall": {"name": "b", "args": {"y": 2}}}])
    );
}

#[test]
fn tool_call_deltas_without_a_function_are_ignored() {
    let mut state = GenAiStreamState::default();
    let custom = stream_chunk(
        json!({"tool_calls": [{"index": 0, "type": "custom"}]}),
        None,
        None,
    );
    assert!(translate_chunk(&custom, &mut state).is_none());
    assert!(state.accumulated_tool_calls.is_empty());
    let empty = stream_chunk(
        json!({"tool_calls": [{"index": 0, "function": {"name": null, "arguments": ""}}]}),
        None,
        None,
    );
    assert!(translate_chunk(&empty, &mut state).is_none());
    assert!(state.accumulated_tool_calls.is_empty());
}

#[test]
fn usage_is_added_only_to_the_final_chunk() {
    let mut state = GenAiStreamState::default();
    let usage = json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3});
    let text = stream_chunk(json!({"content": "hi"}), None, Some(usage.clone()));
    let text_value = translated_chunk(&text, &mut state);
    assert!(text_value.get("usageMetadata").is_none());
    let finish = stream_chunk(json!({}), Some("length"), Some(usage));
    let finish_value = translated_chunk(&finish, &mut state);
    assert_eq!(
        finish_value["candidates"][0]["finishReason"],
        json!("MAX_TOKENS")
    );
    assert_eq!(
        finish_value["usageMetadata"],
        json!({"promptTokenCount": 1, "candidatesTokenCount": 2, "totalTokenCount": 3})
    );
}

#[test]
fn an_empty_finish_reason_is_ignored() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(json!({}), Some(""), None);
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn only_the_first_streaming_choice_is_translated() {
    let chunk: Chunk = serde_json::from_value(json!({
        "id": "chunk-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "m",
        "choices": [
            {"index": 0, "delta": {}, "finish_reason": null},
            {"index": 1, "delta": {"content": "second"}, "finish_reason": null},
        ],
    }))
    .expect("a stream chunk");
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[tokio::test]
async fn the_streaming_entry_point_frames_each_chunk_as_sse_data() {
    let frames = framed(vec![
        stream_chunk(json!({"content": "a"}), None, None),
        stream_chunk(json!({}), Some("stop"), None),
    ])
    .await;
    assert_eq!(frames.len(), 2);
    assert_eq!(
        payload(&frames[0])["candidates"][0]["content"]["parts"],
        json!([{"text": "a"}])
    );
    assert_eq!(
        payload(&frames[1])["candidates"][0]["finishReason"],
        json!("STOP")
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame.starts_with("data: ") && frame.ends_with("\n\n"))
    );
}

#[tokio::test]
async fn empty_chunks_are_skipped_between_frames() {
    let frames = framed(vec![
        stream_chunk(json!({"content": "a"}), None, None),
        stream_chunk(json!({}), None, None),
        stream_chunk(json!({"content": "b"}), None, None),
    ])
    .await;
    assert_eq!(frames.len(), 2);
    assert_eq!(
        payload(&frames[1])["candidates"][0]["content"]["parts"],
        json!([{"text": "b"}])
    );
}

#[tokio::test]
async fn each_frame_is_yielded_before_the_next_chunk_is_read() {
    let read = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&read);
    let source = stream::iter(vec![
        stream_chunk(json!({"content": "a"}), None, None),
        stream_chunk(json!({"content": "b"}), None, None),
    ])
    .inspect(move |_chunk| {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .boxed();
    let mut frames = translate_stream(source);
    assert_eq!(read.load(Ordering::SeqCst), 0);
    let first = frames
        .next()
        .await
        .expect("the first frame")
        .expect("a frame");
    assert_eq!(read.load(Ordering::SeqCst), 1);
    let text = String::from_utf8(first).expect("utf-8");
    assert_eq!(
        payload(&text)["candidates"][0]["content"]["parts"],
        json!([{"text": "a"}])
    );
}

#[rstest]
fn replays_the_genai_chat_output_corpus(#[files("fuzz/corpus/genai_chat_output/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    if path.file_stem() == Some(OsStr::new("invalid-json")) {
        drop(
            serde_json::from_slice::<Chunk>(&bytes)
                .expect_err("a corpus file whose JSON does not parse"),
        );
        return;
    }
    let chunk =
        serde_json::from_slice::<Chunk>(&bytes).expect("every other corpus file is a chunk");
    let mut state = GenAiStreamState::default();
    drop(translate_chunk(&chunk, &mut state));
}
