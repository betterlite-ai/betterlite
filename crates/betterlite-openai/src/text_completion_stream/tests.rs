#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, StreamChoice, StreamChunk, StreamDelta, TokenDetails, Usage,
};
use rstest::rstest;
use serde_json::Value;

use super::{
    DONE_FRAME, TextStreamDecodeError, chat_chunk, decode_chat_events, shaped_chunk, without_nulls,
};
use crate::stream::SseParser;

const CORPUS: &str = "fuzz/corpus/text_stream";

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("json fixture")
}

fn usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

fn chunk(delta: StreamDelta) -> StreamChunk {
    StreamChunk {
        id: String::from("cmpl-1"),
        object: String::from("chat.completion.chunk"),
        created: 1_700_000_000,
        model: String::from("upstream"),
        system_fingerprint: None,
        choices: vec![StreamChoice {
            index: 0,
            delta,
            finish_reason: None,
            logprobs: None,
            extra: JsonObject::new(),
        }],
        usage: None,
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn content(text: &str) -> StreamDelta {
    StreamDelta {
        content: Some(text.to_owned()),
        role: None,
        function_call: None,
        tool_calls: None,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content: None,
        thinking_blocks: None,
        provider_specific_fields: None,
        extra: JsonObject::new(),
    }
}

// The chat chunk a text chunk becomes, the way the chat route writes it
fn emitted(chunk: &StreamChunk, include_usage: bool) -> Value {
    chat_chunk(chunk, "client", include_usage)
        .expect("ok")
        .expect("chunk")
}

// The whole-buffer form the corpus replay decodes: every event, the final unterminated one included
fn decode_chat_bytes(
    bytes: &[u8],
    model: &str,
    include_usage: bool,
) -> Result<Vec<Json>, TextStreamDecodeError> {
    let mut parser = SseParser::default();
    let mut events = parser.push(bytes)?;
    events.extend(parser.finish());
    decode_chat_events(&events, model, include_usage)
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    WithoutNulls,
    ShapedRestamp,
    ShapedDetailsFilled,
    ShapedDetailsKept,
    ChatSilent,
    ChatUsageAsked,
    ChatUsageUnasked,
    ChatRoleContent,
    ChatFinish,
    ChatDone,
    ChatSplitBoundaries,
    ChatInvalidUtf8,
    ChatMalformedJson,
}

#[rstest]
#[case::without_nulls(Scenario::WithoutNulls)]
#[case::shaped_restamp(Scenario::ShapedRestamp)]
#[case::shaped_details_filled(Scenario::ShapedDetailsFilled)]
#[case::shaped_details_kept(Scenario::ShapedDetailsKept)]
#[case::chat_silent(Scenario::ChatSilent)]
#[case::chat_usage_asked(Scenario::ChatUsageAsked)]
#[case::chat_usage_unasked(Scenario::ChatUsageUnasked)]
#[case::chat_role_content(Scenario::ChatRoleContent)]
#[case::chat_finish(Scenario::ChatFinish)]
#[case::chat_done(Scenario::ChatDone)]
#[case::chat_split_boundaries(Scenario::ChatSplitBoundaries)]
#[case::chat_invalid_utf8(Scenario::ChatInvalidUtf8)]
#[case::chat_malformed_json(Scenario::ChatMalformedJson)]
fn text_streaming_text_stream_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::WithoutNulls => without_nulls_scenario(),
        Scenario::ShapedRestamp => shaped_restamp_scenario(),
        Scenario::ShapedDetailsFilled => shaped_details_filled_scenario(),
        Scenario::ShapedDetailsKept => shaped_details_kept_scenario(),
        Scenario::ChatSilent => chat_silent_scenario(),
        Scenario::ChatUsageAsked => chat_usage_asked_scenario(),
        Scenario::ChatUsageUnasked => chat_usage_unasked_scenario(),
        Scenario::ChatRoleContent => chat_role_content_scenario(),
        Scenario::ChatFinish => chat_finish_scenario(),
        Scenario::ChatDone => chat_done_scenario(),
        Scenario::ChatSplitBoundaries => chat_split_boundaries_scenario(),
        Scenario::ChatInvalidUtf8 => chat_invalid_utf8_scenario(),
        Scenario::ChatMalformedJson => chat_malformed_json_scenario(),
    }
}

fn without_nulls_scenario() {
    let cleaned = without_nulls(fixture(
        r#"{"a": null, "b": {"c": null, "d": 1}, "e": [null, {"f": null}], "g": "keep"}"#,
    ));
    assert_eq!(
        cleaned,
        fixture(r#"{"b": {"d": 1}, "e": [null, {}], "g": "keep"}"#)
    );
}

fn shaped_restamp_scenario() {
    let value = shaped_chunk(chunk(content("hi")), "client").expect("value");
    assert_eq!(value.get("model"), Some(&Value::from("client")));
    assert_eq!(
        value.get("object"),
        Some(&Value::from("chat.completion.chunk"))
    );
}

fn shaped_details_filled_scenario() {
    let mut shaped = chunk(content("hi"));
    shaped.usage = Some(usage(3, 2));
    let value = shaped_chunk(shaped, "client").expect("value");
    assert_eq!(
        value.pointer("/usage/completion_tokens_details/reasoning_tokens"),
        Some(&Value::from(0_i64))
    );
    assert_eq!(
        value.pointer("/usage/completion_tokens_details/text_tokens"),
        Some(&Value::from(2_i64))
    );
}

fn shaped_details_kept_scenario() {
    let mut shaped = chunk(content("hi"));
    let mut given = usage(3, 2);
    given.completion_tokens_details = Some(TokenDetails {
        cached_tokens: Some(1),
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cache_creation_token_details: None,
        extra: JsonObject::new(),
    });
    shaped.usage = Some(given);
    let value = shaped_chunk(shaped, "client").expect("value");
    assert_eq!(
        value.pointer("/usage/completion_tokens_details/cached_tokens"),
        Some(&Value::from(1_i64))
    );
    assert_eq!(
        value.pointer("/usage/completion_tokens_details/reasoning_tokens"),
        None
    );
}

fn chat_silent_scenario() {
    assert_eq!(
        chat_chunk(&chunk(StreamDelta::default()), "client", false).expect("ok"),
        None
    );
    let mut with_usage = chunk(StreamDelta::default());
    with_usage.usage = Some(usage(3, 2));
    assert_eq!(chat_chunk(&with_usage, "client", false).expect("ok"), None);
}

fn chat_usage_asked_scenario() {
    let mut with_usage = chunk(StreamDelta::default());
    with_usage.usage = Some(usage(3, 2));
    let value = emitted(&with_usage, true);
    assert_eq!(
        value.pointer("/usage/total_tokens"),
        Some(&Value::from(5_i64))
    );
    assert_eq!(value.pointer("/choices/0/delta"), Some(&fixture("{}")));
}

fn chat_usage_unasked_scenario() {
    let mut with_usage = chunk(content("hi"));
    with_usage.usage = Some(usage(3, 2));
    let value = emitted(&with_usage, false);
    assert_eq!(value.get("usage"), None);
}

fn chat_role_content_scenario() {
    let first = chunk(StreamDelta {
        content: Some(String::from("Hello")),
        role: Some(String::from("assistant")),
        function_call: None,
        tool_calls: None,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content: None,
        thinking_blocks: None,
        provider_specific_fields: None,
        extra: JsonObject::new(),
    });
    let value = emitted(&first, false);
    assert_eq!(
        value.pointer("/choices/0/delta"),
        Some(&fixture(r#"{"role": "assistant", "content": "Hello"}"#))
    );
    assert_eq!(value.pointer("/choices/0/finish_reason"), None);
    assert_eq!(value.get("id"), Some(&Value::from("cmpl-1")));
    assert_eq!(
        value.get("object"),
        Some(&Value::from("chat.completion.chunk"))
    );
    assert_eq!(value.get("model"), Some(&Value::from("client")));
}

fn chat_finish_scenario() {
    let mut last = chunk(content("hi"));
    last.choices[0].finish_reason = Some(String::from("stop"));
    let value = emitted(&last, false);
    assert_eq!(
        value.pointer("/choices/0/finish_reason"),
        Some(&Value::from("stop"))
    );
    assert_eq!(
        value.pointer("/choices/0/delta"),
        Some(&fixture(r#"{"content": "hi"}"#))
    );
}

fn chat_done_scenario() {
    let first = fixture(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"a"}}]}"#,
    );
    let last = fixture(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
    );
    let events = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
    let decoded = decode_chat_bytes(events.as_bytes(), "client", false).expect("decode");
    assert_eq!(decoded.len(), 2);
    let first_chunk = value_of(&decoded[0]);
    assert_eq!(
        first_chunk.pointer("/choices/0/delta/content"),
        Some(&Value::from("a"))
    );
    assert_eq!(first_chunk.get("model"), Some(&Value::from("client")));
    assert_eq!(
        value_of(&decoded[1]).pointer("/choices/0/finish_reason"),
        Some(&Value::from("stop"))
    );
}

fn chat_split_boundaries_scenario() {
    let first = fixture(
        "{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"h\\u00e9llo\"}}]}",
    );
    let last = fixture(
        "{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" w\\u00f6rld\"}}]}",
    );
    let events = format!("data: {first}\n\ndata: {last}\n\n");
    let whole = decode_chat_bytes(events.as_bytes(), "client", false).expect("whole");
    for boundary in 1..events.len() {
        let (left, right) = events.as_bytes().split_at(boundary);
        let mut parser = SseParser::default();
        let mut split = parser
            .push(left)
            .map_err(TextStreamDecodeError::from)
            .and_then(|parsed| decode_chat_events(&parsed, "client", false))
            .expect("left");
        split.extend(
            parser
                .push(right)
                .map_err(TextStreamDecodeError::from)
                .and_then(|parsed| decode_chat_events(&parsed, "client", false))
                .expect("right"),
        );
        split.extend(decode_chat_events(&parser.finish(), "client", false).expect("finish"));
        assert_eq!(split, whole, "boundary {boundary}");
    }
}

fn chat_invalid_utf8_scenario() {
    let mut bytes =
        b"data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"".to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(b"\"}}]}\n\n");
    decode_chat_bytes(&bytes, "client", false).unwrap_err();
}

fn chat_malformed_json_scenario() {
    decode_chat_bytes(b"data: {\"id\": \n\n", "client", false).unwrap_err();
}

#[test]
fn done_frame_is_the_last_event_of_a_text_stream() {
    assert_eq!(DONE_FRAME, "data: [DONE]\n\n");
}

#[test]
fn replays_the_text_stream_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS);
    let mut files: Vec<_> = fs::read_dir(&folder)
        .expect("corpus folder")
        .map(|entry| entry.expect("entry").path())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the corpus holds no file");
    let mut failures = Vec::new();
    files
        .into_iter()
        .try_for_each(|file| {
            failures.extend(boundary_failures(&file)?);
            Ok::<(), io::Error>(())
        })
        .expect("corpus files");
    assert!(failures.is_empty(), "{failures:?}");
}

fn boundary_failures(file: &Path) -> Result<Vec<String>, io::Error> {
    let bytes = fs::read(file)?;
    let whole = decode_chat_bytes(&bytes, "client", true);
    Ok((1..=bytes.len().min(8))
        .filter(|boundary| {
            let (left, right) = bytes.split_at(*boundary);
            let split = split_decode(left, right);
            !match (&split, &whole) {
                (Ok(split_chunks), Ok(whole_chunks)) => split_chunks == whole_chunks,
                (Err(_), Err(_)) => true,
                (Ok(_), Err(_)) | (Err(_), Ok(_)) => false,
            }
        })
        .map(|boundary| format!("{file:?} boundary {boundary}"))
        .collect())
}

fn value_of(json: &Json) -> Value {
    serde_json::to_value(json).expect("json")
}

fn split_decode(left: &[u8], right: &[u8]) -> Result<Vec<Json>, TextStreamDecodeError> {
    let mut parser = SseParser::default();
    let mut chunks = parser
        .push(left)
        .map_err(TextStreamDecodeError::from)
        .and_then(|parsed| decode_chat_events(&parsed, "client", true))?;
    chunks.extend(
        parser
            .push(right)
            .map_err(TextStreamDecodeError::from)
            .and_then(|parsed| decode_chat_events(&parsed, "client", true))?,
    );
    chunks.extend(decode_chat_events(&parser.finish(), "client", true)?);
    Ok(chunks)
}
