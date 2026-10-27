use super::*;
use serde_json::json;
use std::fs;
use std::path::Path;

fn chunk(delta: Delta) -> Chunk {
    Chunk {
        choices: vec![Choice { delta: Some(delta) }],
    }
}

#[test]
fn content_and_reasoning() {
    let chunks = vec![
        chunk(Delta {
            content: Some(json!("a")),
            reasoning_content: Some(json!("r")),
            ..Delta::default()
        }),
        chunk(Delta {
            content: Some(json!(null)),
            reasoning_content: Some(json!("s")),
            ..Delta::default()
        }),
    ];
    assert_eq!(
        get_combined_content(&chunks, ContentField::Content),
        Ok("a".to_owned())
    );
    assert_eq!(
        get_combined_content(&chunks, ContentField::Reasoning),
        Ok("rs".to_owned())
    );
    assert_eq!(
        get_combined_content(
            &[chunk(Delta {
                content: Some(json!([])),
                ..Delta::default()
            })],
            ContentField::Content
        ),
        Err(AssemblyError::NonStringContent)
    );
}

#[test]
fn thinking_signature_boundaries() {
    let chunks = vec![chunk(Delta {
        thinking_blocks: Some(json!([
            {"type":"thinking","thinking":"a"}, {"thinking":"b","signature":"s"},
            {"thinking":"lost"}, {"type":"redacted_thinking","data":"secret"},
            {"type":"thinking","signature":"empty"}, {"thinking":"tail"}
        ])),
        ..Delta::default()
    })];
    assert_eq!(
        get_combined_thinking_content(&chunks),
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
            }
        ])
    );
}

#[test]
fn audio_fragments_and_expiry() {
    let chunks = vec![
        chunk(Delta {
            audio: Some(AudioDelta {
                data: Some(json!("YQ==")),
                transcript: Some(json!("a")),
                expires_at: Some(json!(0)),
                id: Some(json!("one")),
            }),
            ..Delta::default()
        }),
        chunk(Delta {
            audio: Some(AudioDelta {
                data: Some(json!("Yg==")),
                transcript: Some(json!("b")),
                expires_at: Some(json!(-2)),
                id: Some(json!("")),
            }),
            ..Delta::default()
        }),
    ];
    assert_eq!(
        get_combined_audio_content(&chunks, 100).unwrap(),
        AudioResponse {
            data: "YWI=".to_owned(),
            expires_at: json!(-2),
            transcript: "ab".to_owned(),
            id: Some(String::new())
        }
    );
}

#[test]
fn audio_base64_decoder() {
    assert_eq!(decode_stream_audio_fragment("YQ==\n", 1).unwrap(), b"a");
    assert_eq!(decode_stream_audio_fragment("", 1).unwrap(), b"");
    assert!(decode_stream_audio_fragment("YQ=", 1).is_err());
    assert!(decode_stream_audio_fragment("YWFh", 2).is_err());
}

#[test]
fn replays_the_stream_content_assembly_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/stream_content_assembly");
    for entry in fs::read_dir(root).unwrap() {
        let fragment = fs::read_to_string(entry.unwrap().path()).unwrap();
        let _ = decode_stream_audio_fragment(&fragment, DEFAULT_AUDIO_LIMIT);
    }
}
