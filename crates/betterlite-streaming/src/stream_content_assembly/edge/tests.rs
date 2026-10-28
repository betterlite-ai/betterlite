#![cfg(test)]

use super::wire::{AudioResponse, Chunk, ExpiryValue, ThinkingBlock};
use super::{
    AssemblyError, ContentField, DEFAULT_AUDIO_LIMIT, get_combined_audio_content,
    get_combined_content, get_combined_thinking_content,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;

#[rstest]
#[case::empty("[]")]
#[case::no_choices(r#"[{"choices":[]}]"#)]
#[case::missing_delta(r#"[{"choices":[{}]}]"#)]
#[case::empty_blocks(r#"[{"choices":[{"delta":{"thinking_blocks":[]}}]}]"#)]
#[case::non_list(r#"[{"choices":[{"delta":{"thinking_blocks":"ignored"}}]}]"#)]
#[case::unsigned(
    r#"[{"choices":[{"delta":{"thinking_blocks":[{"thinking":"tail","signature":""}]}}]}]"#
)]
fn incomplete_thinking(#[case] source: &str) {
    let chunks: Vec<Chunk> = serde_json::from_str(source).unwrap();
    assert_eq!(get_combined_thinking_content(&chunks), Ok(None));
}

#[test]
fn empty_redaction_discards_text() {
    let chunks: Vec<Chunk> = serde_json::from_str(
        r#"[{"choices":[{"delta":{"thinking_blocks":[
        {"thinking":"discard"},{"type":"redacted_thinking","data":""},
        {"type":"unknown","thinking":"","signature":"s"}
    ]}}]}]"#,
    )
    .unwrap();
    assert_eq!(
        get_combined_thinking_content(&chunks),
        Ok(Some(vec![ThinkingBlock::Thinking {
            thinking: String::new(),
            signature: "s".to_owned(),
        }]))
    );
}

#[test]
fn absent_audio_has_defaults() {
    assert_eq!(
        get_combined_audio_content(&[], 123),
        Ok(AudioResponse {
            data: String::new(),
            expires_at: ExpiryValue::Signed(3723),
            transcript: String::new(),
            id: None,
        })
    );
}

#[test]
fn text_follows_every_choice() {
    let chunks: Vec<Chunk> = serde_json::from_str(
        r#"[{"choices":[{},
        {"delta":{"content":"first","reasoning_content":null}},
        {"delta":{"content":"second","reasoning_content":"reason"}}]}]"#,
    )
    .unwrap();
    assert_eq!(
        get_combined_content(&chunks, ContentField::Content),
        Ok("firstsecond".to_owned())
    );
    assert_eq!(
        get_combined_content(&chunks, ContentField::Reasoning),
        Ok("reason".to_owned())
    );
}

#[test]
fn audio_limit_applies_across_fragments() {
    let fragment = STANDARD.encode(vec![0; DEFAULT_AUDIO_LIMIT]);
    let chunks: Vec<Chunk> = serde_json::from_str(&format!(
        r#"[{{"choices":[
        {{"delta":{{"audio":{{"data":"{fragment}"}}}}}},
        {{"delta":{{"audio":{{"data":"YQ=="}}}}}}
    ]}}]"#
    ))
    .unwrap();
    assert_eq!(
        get_combined_audio_content(&chunks, 100),
        Err(AssemblyError::AudioTooLarge { limit: 0 })
    );
}

#[test]
fn audio_id_is_bounded() {
    let text = "x".repeat(DEFAULT_AUDIO_LIMIT + 1);
    let chunks: Vec<Chunk> = serde_json::from_str(&format!(
        r#"[{{"choices":[{{"delta":{{"audio":{{"id":"{text}"}}}}}}]}}]"#
    ))
    .unwrap();
    assert_eq!(
        get_combined_audio_content(&chunks, 100),
        Err(AssemblyError::OutputTooLarge)
    );
}
