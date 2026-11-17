#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{
    StreamChoice, StreamChunk, StreamDelta, TokenDetails, Usage,
};
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    DONE_FRAME, TextStreamDecodeError, chat_chunk, decode_chat_bytes, decode_chat_events,
    shaped_chunk, without_nulls,
};
use crate::stream::SseParser;

const CORPUS: &str = "fuzz/corpus/text_stream";

fn usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        ..Usage::default()
    }
}

fn chunk(delta: StreamDelta) -> StreamChunk {
    StreamChunk {
        id: String::from("cmpl-1"),
        created: 1_700_000_000,
        model: String::from("upstream"),
        choices: vec![StreamChoice {
            delta,
            ..StreamChoice::default()
        }],
        ..StreamChunk::default()
    }
}

fn content(text: &str) -> StreamDelta {
    StreamDelta {
        content: Some(text.to_owned()),
        ..StreamDelta::default()
    }
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
        Scenario::WithoutNulls => {
            let cleaned = without_nulls(json!({
                "a": null, "b": {"c": null, "d": 1}, "e": [null, {"f": null}], "g": "keep"
            }));
            assert_eq!(
                cleaned,
                json!({"b": {"d": 1}, "e": [null, {}], "g": "keep"})
            );
        }
        Scenario::ShapedRestamp => {
            let value = shaped_chunk(chunk(content("hi")), "client").expect("value");
            assert_eq!(value.get("model"), Some(&json!("client")));
            assert_eq!(value.get("object"), Some(&json!("chat.completion.chunk")));
        }
        Scenario::ShapedDetailsFilled => {
            let mut shaped = chunk(content("hi"));
            shaped.usage = Some(usage(3, 2));
            let value = shaped_chunk(shaped, "client").expect("value");
            assert_eq!(
                value.pointer("/usage/completion_tokens_details/reasoning_tokens"),
                Some(&json!(0))
            );
            assert_eq!(
                value.pointer("/usage/completion_tokens_details/text_tokens"),
                Some(&json!(2))
            );
        }
        Scenario::ShapedDetailsKept => {
            let mut shaped = chunk(content("hi"));
            let mut given = usage(3, 2);
            given.completion_tokens_details = Some(TokenDetails {
                cached_tokens: Some(1),
                ..TokenDetails::default()
            });
            shaped.usage = Some(given);
            let value = shaped_chunk(shaped, "client").expect("value");
            assert_eq!(
                value.pointer("/usage/completion_tokens_details/cached_tokens"),
                Some(&json!(1))
            );
            assert_eq!(
                value.pointer("/usage/completion_tokens_details/reasoning_tokens"),
                None
            );
        }
        Scenario::ChatSilent => {
            assert_eq!(
                chat_chunk(&chunk(StreamDelta::default()), "client", false).expect("ok"),
                None
            );
            let mut with_usage = chunk(StreamDelta::default());
            with_usage.usage = Some(usage(3, 2));
            assert_eq!(chat_chunk(&with_usage, "client", false).expect("ok"), None);
        }
        Scenario::ChatUsageAsked => {
            let mut with_usage = chunk(StreamDelta::default());
            with_usage.usage = Some(usage(3, 2));
            let value = chat_chunk(&with_usage, "client", true)
                .expect("ok")
                .expect("chunk");
            assert_eq!(value.pointer("/usage/total_tokens"), Some(&json!(5)));
            assert_eq!(value.pointer("/choices/0/delta"), Some(&json!({})));
        }
        Scenario::ChatUsageUnasked => {
            let mut with_usage = chunk(content("hi"));
            with_usage.usage = Some(usage(3, 2));
            let value = chat_chunk(&with_usage, "client", false)
                .expect("ok")
                .expect("chunk");
            assert_eq!(value.get("usage"), None);
        }
        Scenario::ChatRoleContent => {
            let mut first = chunk(StreamDelta {
                role: Some(String::from("assistant")),
                content: Some(String::from("Hel")),
                ..StreamDelta::default()
            });
            first.model = String::from("upstream");
            let value = chat_chunk(&first, "client", false)
                .expect("ok")
                .expect("chunk");
            assert_eq!(
                value.pointer("/choices/0/delta"),
                Some(&json!({"role": "assistant", "content": "Hel"}))
            );
            assert_eq!(value.pointer("/choices/0/finish_reason"), None);
            assert_eq!(value.get("id"), Some(&json!("cmpl-1")));
            assert_eq!(value.get("object"), Some(&json!("chat.completion.chunk")));
            assert_eq!(value.get("model"), Some(&json!("client")));
        }
        Scenario::ChatFinish => {
            let mut last = chunk(StreamDelta::default());
            last.choices[0].finish_reason = Some(String::from("stop"));
            let value = chat_chunk(&last, "client", false)
                .expect("ok")
                .expect("chunk");
            assert_eq!(
                value.pointer("/choices/0/finish_reason"),
                Some(&json!("stop"))
            );
            assert_eq!(value.pointer("/choices/0/delta"), Some(&json!({})));
        }
        Scenario::ChatDone => {
            let events = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"a"}}]}),
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            );
            let decoded = decode_chat_bytes(events.as_bytes(), "client", false).expect("decode");
            assert_eq!(decoded.len(), 2);
            let first = value_of(&decoded[0]);
            assert_eq!(first.pointer("/choices/0/delta/content"), Some(&json!("a")));
            assert_eq!(first.get("model"), Some(&json!("client")));
            assert_eq!(
                value_of(&decoded[1]).pointer("/choices/0/finish_reason"),
                Some(&json!("stop"))
            );
        }
        Scenario::ChatSplitBoundaries => {
            let events = format!(
                "data: {}\n\ndata: {}\n\n",
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"héllo"}}]}),
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":" wörld"}}]}),
            );
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
                split
                    .extend(decode_chat_events(&parser.finish(), "client", false).expect("finish"));
                assert_eq!(split, whole, "boundary {boundary}");
            }
        }
        Scenario::ChatInvalidUtf8 => {
            let mut bytes =
                b"data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"".to_vec();
            bytes.push(0xff);
            bytes.extend_from_slice(b"\"}}]}\n\n");
            assert!(decode_chat_bytes(&bytes, "client", false).is_err());
        }
        Scenario::ChatMalformedJson => {
            assert!(decode_chat_bytes(b"data: {\"id\": \n\n", "client", false).is_err());
        }
    }
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
                (Ok(split), Ok(whole)) => split == whole,
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
