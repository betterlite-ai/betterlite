#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::completion::error::CompletionError;
use crate::completion::reply::{LegacyReply, decode_legacy_anthropic_reply};
use crate::completion::stream::decode_stream_line;

#[test]
fn replays_the_anthropic_completion_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_completion");
    let sources = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect::<Vec<_>>();
    assert!(sources.len() >= 8);
    for (name, bytes) in &sources {
        let decoded = decode_legacy_anthropic_reply(502, bytes);
        match name.as_str() {
            "valid.json" | "boundary.json" | "error.json" => assert!(decoded.is_ok(), "{name}"),
            "malformed.json" | "truncated.json" | "non-object.json" => {
                assert!(
                    matches!(decoded, Err(CompletionError::Upstream(_))),
                    "{name}"
                );
            }
            "missing-field.json" => {
                assert!(matches!(decoded, Err(CompletionError::MissingField { .. })));
            }
            "wrong-type.json" => {
                assert!(matches!(decoded, Err(CompletionError::FieldType { .. })));
            }
            "invalid-utf8.json" => {
                assert!(matches!(decoded, Err(CompletionError::NotUtf8 { .. })));
            }
            _ => {}
        }
        if let Ok(text) = str::from_utf8(bytes) {
            let line = decode_stream_line(text);
            assert!(line.is_ok() || matches!(line, Err(CompletionError::ChunkNotObject)));
        }
    }
    let body = br#"{"completion":" Hello","stop_reason":"stop_sequence"}"#;
    let expected = LegacyReply::Completion {
        completion: " Hello".to_owned(),
        stop_reason: Some("stop_sequence".to_owned()),
    };
    assert_eq!(decode_legacy_anthropic_reply(200, body).unwrap(), expected);
    let error = decode_legacy_anthropic_reply(429, br#"{"error":"slow down"}"#);
    let message = "slow down".to_owned();
    assert_eq!(error.unwrap(), LegacyReply::Error { message });
}
