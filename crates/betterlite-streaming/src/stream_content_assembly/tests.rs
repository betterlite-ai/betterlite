#![cfg(test)]

use super::*;
use rstest::rstest;
use std::fs;
use std::io::Error;
use std::path::Path;

fn parse_chunks(source: &str) -> Vec<Chunk> {
    let parsed: Vec<Chunk> = serde_json::from_str(source).unwrap();
    if parsed.is_empty() {
        Vec::new()
    } else {
        parsed
    }
}

#[rstest]
fn case_1_content_and_reasoning() {
    let parsed_chunks = parse_chunks(
        r#"[
        {"choices":[{"delta":{"content":"a","reasoning_content":"r"}}]},
        {"choices":[{"delta":{"content":null,"reasoning_content":"s"}}]}
    ]"#,
    );
    assert_eq!(
        get_combined_content(&parsed_chunks, ContentField::Content),
        Ok("a".to_owned())
    );
    assert_eq!(
        get_combined_content(&parsed_chunks, ContentField::Reasoning),
        Ok("rs".to_owned())
    );
    let invalid = parse_chunks(r#"[{"choices":[{"delta":{"content":[]}}]}]"#);
    assert_eq!(
        get_combined_content(&invalid, ContentField::Content),
        Err(AssemblyError::NonStringContent)
    );
}

#[rstest]
fn case_1_thinking_signature_boundaries() {
    let parsed_chunks = parse_chunks(
        r#"[{"choices":[{"delta":{"thinking_blocks":[
        {"type":"thinking","thinking":"a"},{"thinking":"b","signature":"s"},
        {"thinking":"lost"},{"type":"redacted_thinking","data":"secret"},
        {"type":"thinking","signature":"empty"},{"thinking":"tail"}
    ]}}]}]"#,
    );
    assert_eq!(
        get_combined_thinking_content(&parsed_chunks),
        Some(vec![
            ThinkingBlock::Thinking {
                thinking: "ab".to_owned(),
                signature: "s".to_owned()
            },
            ThinkingBlock::Redacted {
                data: "secret".to_owned()
            },
            ThinkingBlock::Thinking {
                thinking: String::new(),
                signature: "empty".to_owned()
            },
        ])
    );
}

#[rstest]
fn case_1_audio_fragments_and_expiry() {
    let parsed_chunks = parse_chunks(
        r#"[
        {"choices":[{"delta":{"audio":{"data":"YQ==","transcript":"a","expires_at":0,"id":"one"}}}]},
        {"choices":[{"delta":{"audio":{"data":"Yg==","transcript":"b","expires_at":-2,"id":""}}}]}
    ]"#,
    );
    assert_eq!(
        get_combined_audio_content(&parsed_chunks, 100).unwrap(),
        AudioResponse {
            data: "YWI=".to_owned(),
            expires_at: ExpiryValue::Signed(-2),
            transcript: "ab".to_owned(),
            id: Some(String::new()),
        }
    );
}

#[rstest]
fn case_1_audio_base64_decoder() {
    assert_eq!(decode_stream_audio_fragment("YQ==\n", 1).unwrap(), b"a");
    assert_eq!(decode_stream_audio_fragment("", 1).unwrap(), b"");
    decode_stream_audio_fragment("YQ=", 1).unwrap_err();
    decode_stream_audio_fragment("YWFh", 2).unwrap_err();
}

#[test]
fn replays_the_stream_content_assembly_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/stream_content_assembly");
    fs::read_dir(root)
        .unwrap()
        .try_for_each(|entry| {
            let fragment = fs::read_to_string(entry.unwrap().path()).unwrap();
            drop(decode_stream_audio_fragment(&fragment, DEFAULT_AUDIO_LIMIT));
            Ok::<(), Error>(())
        })
        .unwrap();
}
