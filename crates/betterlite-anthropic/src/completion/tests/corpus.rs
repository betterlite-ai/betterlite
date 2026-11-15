#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::completion::error::CompletionError;
use crate::completion::reply::{LegacyCompletion, decode_legacy_anthropic_reply};

#[rstest]
fn replays_the_anthropic_completion_corpus(
    #[files("fuzz/corpus/anthropic_completion/*")] path: PathBuf,
) {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let decoded = decode_legacy_anthropic_reply(502, &bytes);
    match name {
        "valid.json" | "boundary.json" => assert!(decoded.is_ok(), "{name}"),
        "error.json" | "malformed.json" | "truncated.json" | "non-object.json" => {
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
}

#[test]
fn decodes_known_replies() {
    let body = br#"{"completion":" Hello","stop_reason":"stop_sequence"}"#;
    let expected = LegacyCompletion {
        completion: " Hello".to_owned(),
        stop_reason: Some("stop_sequence".to_owned()),
    };
    assert_eq!(decode_legacy_anthropic_reply(200, body).unwrap(), expected);
    let error = decode_legacy_anthropic_reply(429, br#"{"error":"slow down"}"#).unwrap_err();
    let CompletionError::Upstream(upstream) = error else {
        panic!("an error reply is not an upstream error");
    };
    assert_eq!(upstream.status_code, 429);
    assert_eq!(upstream.message, "slow down");
}
