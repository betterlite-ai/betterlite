#![cfg(test)]

use super::wire::{
    AudioDelta, AudioResponse, Choice, Chunk, ChunkText, Delta, ExpiryValue, ThinkingBlock,
};
use super::{
    AssemblyError, ContentField, DEFAULT_AUDIO_LIMIT, decode_stream_audio_fragment,
    get_combined_audio_content, get_combined_content, get_combined_thinking_content,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;
use std::fs;
use std::io::Error;
use std::path::Path;

#[rstest]
#[case::content_and_reasoning(content_and_reasoning)]
#[case::thinking_signature_boundaries(thinking_signature_boundaries)]
#[case::audio_fragments_and_expiry(audio_fragments_and_expiry)]
#[case::audio_base64_decoder(audio_base64_decoder)]
fn stream_content_assembly_cases(#[case] verify: fn()) {
    verify();
}

fn parse_chunks(source: &str) -> Vec<Chunk> {
    let parsed: Vec<Chunk> = serde_json::from_str(source).unwrap();
    if parsed.is_empty() {
        Vec::new()
    } else {
        parsed
    }
}

fn content_and_reasoning() {
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

fn thinking_signature_boundaries() {
    let parsed_chunks = parse_chunks(
        r#"[{"choices":[{"delta":{"thinking_blocks":[
        {"type":"thinking","thinking":"a"},{"thinking":"b","signature":"s"},
        {"thinking":"lost"},{"type":"redacted_thinking","data":"secret"},
        {"type":"thinking","signature":"empty"},{"thinking":"tail"}
    ]}}]}]"#,
    );
    assert_eq!(
        get_combined_thinking_content(&parsed_chunks).unwrap(),
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

fn audio_fragments_and_expiry() {
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
    let zero_expiry = parse_chunks(r#"[{"choices":[{"delta":{"audio":{"expires_at":0}}}]}]"#);
    assert_eq!(
        get_combined_audio_content(&zero_expiry, 100)
            .unwrap()
            .expires_at,
        ExpiryValue::Signed(3700)
    );
}

fn audio_base64_decoder() {
    assert_eq!(decode_stream_audio_fragment("YQ==\n", 1).unwrap(), b"a");
    assert_eq!(decode_stream_audio_fragment("", 1).unwrap(), b"");
    decode_stream_audio_fragment("YQ=", 1).unwrap_err();
    decode_stream_audio_fragment("YWFh", 2).unwrap_err();

    let fragment = STANDARD.encode(vec![0; DEFAULT_AUDIO_LIMIT]);
    let chunks = vec![Chunk {
        choices: vec![Choice {
            delta: Some(Delta {
                audio: Some(AudioDelta {
                    data: Some(ChunkText::Text(fragment)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }],
    }];
    let result = get_combined_audio_content(&chunks, 0);
    assert_eq!(result.map(|_| ()), Ok(()));

    let medium_fragment = STANDARD.encode(vec![0; 0x0040_0000]);
    let medium_chunks = vec![Chunk {
        choices: vec![Choice {
            delta: Some(Delta {
                audio: Some(AudioDelta {
                    data: Some(ChunkText::Text(medium_fragment)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }],
    }];
    assert_eq!(
        get_combined_audio_content(&medium_chunks, 0).map(|_| ()),
        Ok(())
    );
}

#[rstest]
#[case::excess_padding("YQ===", Some(&b"a"[..]))]
#[case::interior_padding("Y=Q==", Some(&b"a"[..]))]
#[case::trailing_bits("YR==", Some(&b"a"[..]))]
#[case::two_bytes("YWI=", Some(&b"ab"[..]))]
#[case::three_bytes("YWJj", Some(&b"abc"[..]))]
#[case::plus_slash_digits("+/09", Some(&[251,253,61][..]))]
#[case::three_with_padding("YWJj=", Some(&b"abc"[..]))]
#[case::missing_second_padding("YWI", None)]
#[case::padding_only("====", Some(&b""[..]))]
#[case::ignored_symbols("Y!Q=!=", Some(&b"a"[..]))]
#[case::non_ascii("\u{e9}YQ==", None)]
#[case::missing_padding("YQ=", None)]
#[case::data_after_padding("YQ==Yg==", Some(&b"a\x06 "[..]))]
fn python_base64_fragments(#[case] fragment: &str, #[case] expected: Option<&[u8]>) {
    assert_eq!(
        decode_stream_audio_fragment(fragment, 16).ok().as_deref(),
        expected
    );
}

#[rstest]
#[case::boolean_true("true", 1)]
#[case::boolean_false("false", 3700)]
#[case::non_integer("1.5", 3700)]
#[case::non_numeric(r#""later""#, 3700)]
fn audio_expiry_types(#[case] expiry: &str, #[case] expected: i64) {
    let chunks = parse_chunks(&format!(
        r#"[{{"choices":[{{"delta":{{"audio":{{"expires_at":{expiry}}}}}}}]}}]"#
    ));
    assert_eq!(
        get_combined_audio_content(&chunks, 100).unwrap().expires_at,
        ExpiryValue::Signed(expected)
    );
}

#[rstest]
#[case::thinking("thinking", "thinking")]
#[case::signature("signature", "thinking")]
#[case::redacted("data", "redacted_thinking")]
fn thinking_output_is_bounded(#[case] field: &str, #[case] kind: &str) {
    let text = "x".repeat(DEFAULT_AUDIO_LIMIT + 1);
    let chunks = parse_chunks(&format!(
        r#"[{{"choices":[{{"delta":{{"thinking_blocks":[{{"type":"{kind}","{field}":"{text}"}}]}}}}]}}]"#
    ));
    assert_eq!(
        get_combined_thinking_content(&chunks),
        Err(AssemblyError::OutputTooLarge)
    );
}

#[test]
fn text_limit_applies_across_choices() {
    let text = "x".repeat(DEFAULT_AUDIO_LIMIT);
    let mut chunks = parse_chunks(&format!(
        r#"[{{"choices":[{{"delta":{{"content":"{text}"}}}}]}}]"#
    ));
    assert_eq!(
        get_combined_content(&chunks, ContentField::Content).unwrap(),
        text
    );
    chunks.extend(parse_chunks(r#"[{"choices":[{"delta":{"content":"x"}}]}]"#));
    assert_eq!(
        get_combined_content(&chunks, ContentField::Content),
        Err(AssemblyError::OutputTooLarge)
    );
}

#[rstest]
#[case::empty("", 0, Some(&b""[..]))]
#[case::zero_limit("YQ==", 0, None)]
#[case::exact_two("YWI=", 2, Some(&b"ab"[..]))]
#[case::over_two("YWJj", 2, None)]
fn decoder_output_limit(
    #[case] fragment: &str,
    #[case] limit: usize,
    #[case] expected: Option<&[u8]>,
) {
    let result = decode_stream_audio_fragment(fragment, limit);
    match expected {
        Some(bytes) => assert_eq!(result.unwrap(), bytes),
        None => assert_eq!(result, Err(AssemblyError::AudioTooLarge { limit })),
    }
}

#[rstest]
#[case::content("content")]
#[case::reasoning("reasoning_content")]
#[case::transcript("transcript")]
fn assembled_text_is_bounded(#[case] field: &str) {
    let text = "x".repeat(DEFAULT_AUDIO_LIMIT + 1);
    let delta = if field == "transcript" {
        format!(r#"{{"audio":{{"transcript":"{text}"}}}}"#)
    } else {
        format!(r#"{{"{field}":"{text}"}}"#)
    };
    let chunks = parse_chunks(&format!(r#"[{{"choices":[{{"delta":{delta}}}]}}]"#));
    let result = match field {
        "content" => get_combined_content(&chunks, ContentField::Content).map(|_| ()),
        "reasoning_content" => get_combined_content(&chunks, ContentField::Reasoning).map(|_| ()),
        _ => get_combined_audio_content(&chunks, 100).map(|_| ()),
    };
    assert!(result.is_err());
}

#[test]
fn replays_the_stream_content_assembly_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/stream_content_assembly");
    let entries: Vec<_> = fs::read_dir(root).unwrap().collect();
    assert!(!entries.is_empty());
    entries
        .into_iter()
        .try_for_each(|entry| {
            let path = entry.unwrap().path();
            let fragment = fs::read_to_string(&path)?;
            let result = decode_stream_audio_fragment(&fragment, 1);
            match path.file_name().unwrap().to_str().unwrap() {
                "valid" | "excess-padding" | "interior-padding" | "trailing-bits" => {
                    assert_eq!(result.unwrap(), b"a");
                }
                "boundary" => assert_eq!(result.unwrap(), b""),
                "over-limit" => assert_eq!(result, Err(AssemblyError::AudioTooLarge { limit: 1 })),
                "malformed" | "truncated" => {
                    assert_eq!(result, Err(AssemblyError::InvalidAudioPadding));
                }
                "non-ascii" => assert_eq!(result, Err(AssemblyError::NonAsciiAudio)),
                name => panic!("missing outcome for corpus input {name}"),
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
