#![cfg(test)]

use std::{collections::BTreeMap, fs, path::PathBuf};

use rstest::rstest;
use serde_json::json;

use super::{CohereStreamVersion, decode_cohere_line, validate_environment};

#[test]
fn v1_prefers_text_to_finish_and_forwards_citations() {
    let chunk = decode_cohere_line(
        CohereStreamVersion::V1,
        br#"{"text":"hi","is_finished":true,"index":"2","citations":[]}"#,
        false,
    )
    .unwrap();
    assert_eq!(chunk.text, "hi");
    assert!(!chunk.is_finished);
    assert_eq!(chunk.index, 2);
    assert_eq!(
        chunk.provider_specific_fields,
        Some(json!({"citations":[]}))
    );
}

#[test]
fn v2_type_takes_precedence_over_message_end() {
    let chunk = decode_cohere_line(CohereStreamVersion::V2, br#"{"type":"content-delta","event":"message-end","delta":{"message":{"content":{"text":"x"}}}}"#, false).unwrap();
    assert_eq!(chunk.text, "x");
    assert!(!chunk.is_finished);
}

#[test]
fn byte_sse_prefix_is_retained_before_json_decode() {
    assert!(decode_cohere_line(CohereStreamVersion::V1, b"data: {\"text\":\"x\"}", true).is_err());
}

#[test]
fn shared_headers_keep_auth_for_empty_key() {
    let mut headers = BTreeMap::from([(String::from("Authorization"), String::from("caller"))]);
    validate_environment(&mut headers, "");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("caller")
    );
    assert_eq!(
        headers.get("Request-Source").map(String::as_str),
        Some("unspecified:litellm")
    );
}

#[rstest]
fn replays_the_cohere_line_corpus(#[files("fuzz/corpus/cohere_line/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _result = decode_cohere_line(CohereStreamVersion::V2, &bytes, true);
}
