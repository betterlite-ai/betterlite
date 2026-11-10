use crate::arguments::json::Json;
use crate::bedrock_processor::error::ProcessorError;
use crate::bedrock_processor::thinking::{append_thinking_blocks, translate_thinking_blocks};

use super::prelude::list;

fn translated(raw: &str) -> Result<Vec<Json>, ProcessorError> {
    translate_thinking_blocks(&list(raw))
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_keeps_a_signature() {
    assert_eq!(
        translated(r#"[{"thinking":"why","signature":"sig"}]"#).unwrap(),
        list(r#"[{"reasoningContent":{"reasoningText":{"text":"why","signature":"sig"}}}]"#)
    );
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_omits_a_null_signature() {
    assert_eq!(
        translated(r#"[{"thinking":"why","signature":null}]"#).unwrap(),
        list(r#"[{"reasoningContent":{"reasoningText":{"text":"why"}}}]"#)
    );
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_defaults_a_missing_text_to_empty()
{
    assert_eq!(
        translated("[{}]").unwrap(),
        list(r#"[{"reasoningContent":{"reasoningText":{"text":""}}}]"#)
    );
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_rejects_a_non_string_thinking() {
    assert!(matches!(
        translated(r#"[{"thinking":5}]"#).unwrap_err(),
        ProcessorError::ThinkingNotText { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_rejects_a_non_object_block() {
    assert!(matches!(
        translated("[5]").unwrap_err(),
        ProcessorError::ThinkingNotObject { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_thinking_translation_rejects_a_non_string_signature() {
    assert!(matches!(
        translated(r#"[{"thinking":"why","signature":5}]"#).unwrap_err(),
        ProcessorError::ThinkingSignatureNotText { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_signed_reasoning_blocks_are_kept() {
    let mut parts = Vec::new();
    let blocks =
        list(r#"[{"reasoningContent":{"reasoningText":{"text":"why","signature":"sig"}}}]"#);
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(parts, blocks);
}

#[test]
fn bedrock_processor_behavior_tests_behavior_unsigned_reasoning_text_becomes_a_text_block() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{"text":"why"}}}]"#);
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(parts, list(r#"[{"text":"why"}]"#));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_blank_unsigned_reasoning_is_dropped() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{"text":"  "}}}]"#);
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(parts, Vec::new());
}

#[test]
fn bedrock_processor_behavior_tests_behavior_reasoning_without_reasoning_text_is_kept() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{},{"reasoningContent":{}}]"#);
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(parts, blocks);
}

#[test]
fn bedrock_processor_behavior_tests_behavior_unsigned_reasoning_text_comes_before_kept_blocks() {
    let mut parts = Vec::new();
    let blocks = list(
        r#"[{"reasoningContent":{"reasoningText":{"text":"plain"}}},
            {"reasoningContent":{"reasoningText":{"text":"signed","signature":"sig"}}}]"#,
    );
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(
        parts,
        list(
            r#"[{"text":"plain"},
                {"reasoningContent":{"reasoningText":{"text":"signed","signature":"sig"}}}]"#
        )
    );
}

#[test]
fn bedrock_processor_behavior_tests_behavior_reasoning_without_text_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{"signature":null}}}]"#);
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningTextWithoutText { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_reasoning_text_that_is_not_text_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{"text":5}}}]"#);
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningTextNotText { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_reasoning_content_that_is_not_an_object_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":"why"}]"#);
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningContentNotObject { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_reasoning_text_that_is_not_an_object_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":"why"}}]"#);
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningTextNotObject { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_a_non_object_reasoning_block_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list("[5]");
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningNotObject { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_a_non_string_reasoning_signature_is_rejected() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{"text":"why","signature":5}}}]"#);
    let error = append_thinking_blocks(&blocks, &mut parts).unwrap_err();
    assert!(matches!(
        error,
        ProcessorError::ReasoningSignatureNotText { block: 0 }
    ));
}

#[test]
fn bedrock_processor_behavior_tests_behavior_empty_reasoning_text_is_kept() {
    let mut parts = Vec::new();
    let blocks = list(r#"[{"reasoningContent":{"reasoningText":{}}}]"#);
    append_thinking_blocks(&blocks, &mut parts).unwrap();
    assert_eq!(parts, blocks);
}
