#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use indexmap::IndexMap;

use crate::completion::error::CompletionError;
use crate::completion::stream::{chunk_parser, decode_stream_line};

fn chunk(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parsed_chunks() {
    let parsed = chunk_parser(&chunk(r#"{"completion":"Hi","stop_reason":null}"#)).unwrap();
    let expected = GenericChunk {
        text: "Hi".to_owned(),
        is_finished: true,
        index: Some(0),
        ..GenericChunk::default()
    };
    assert_eq!(parsed, expected);
    let other = chunk(r#"{"completion":5,"index":3,"stop_reason":"max_tokens"}"#);
    let expected = GenericChunk {
        is_finished: true,
        finish_reason: "max_tokens".to_owned(),
        index: Some(3),
        ..GenericChunk::default()
    };
    assert_eq!(chunk_parser(&other).unwrap(), expected);
    assert_eq!(chunk_parser(&chunk("{}")).unwrap().text, "");
    let null_text = chunk(r#"{"completion":null}"#);
    assert_eq!(chunk_parser(&null_text).unwrap().text, "");
    for invalid in [
        r#"{"index":"x"}"#,
        r#"{"index":1.5}"#,
        r#"{"stop_reason":5}"#,
    ] {
        assert!(chunk_parser(&chunk(invalid)).is_err(), "{invalid}");
    }
}

pub(super) fn stream_lines() {
    let line = decode_stream_line(r#"data: {"completion":"Hi"}"#).unwrap();
    assert_eq!(line.text, "Hi");
    let tight = decode_stream_line(r#"data:{"completion":"Hi"}"#).unwrap();
    assert_eq!(tight.text, "Hi");
    let done = decode_stream_line("data: [DONE]").unwrap();
    assert!(done.is_finished);
    assert_eq!(done.finish_reason, "stop");
    let blank = GenericChunk {
        index: Some(0),
        ..GenericChunk::default()
    };
    for ignored in ["", "{}", "null", "event: ping"] {
        assert_eq!(decode_stream_line(ignored).unwrap(), blank, "{ignored}");
    }
    let scalar = decode_stream_line("[1]");
    assert!(matches!(scalar, Err(CompletionError::ChunkNotObject)));
}
