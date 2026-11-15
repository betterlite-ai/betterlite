#![cfg(test)]

use rstest::rstest;
use std::error::Error as _;

use crate::api::messages_chat_output::content::{anthropic_finish_reason, content_blocks};
use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::output::AnthropicStopReason;
use crate::api::messages_chat_output::response::response_message;
use crate::api::messages_context_clear::result::{CompactionBlock, CompactionKind, PolyfillResult};

use super::support::{RESPONSE_ID, assert_rendered, blocks, choices, polyfill_result, response};

#[test]
fn ordered_blocks_per_choice() {
    let source = r#"[{
        "message": {
            "content": "hi",
            "refusal": "no",
            "provider_specific_fields": {"compaction_blocks": [
                {"type": "compaction", "content": "sum", "signature": "cs"},
                {"type": "text", "text": "not a block"}
            ]},
            "thinking_blocks": [{"type": "thinking", "thinking": "why", "signature": "sig"}],
            "tool_calls": [{"id": "functions.Bash:0", "function": {"name": "Bash", "arguments": "{\"x\": 1}"}}]
        }
    }]"#;
    assert_rendered(
        &blocks(source),
        r#"[
            {"type": "compaction", "content": "sum", "signature": "cs"},
            {"type": "thinking", "thinking": "why", "signature": "sig"},
            {"type": "text", "text": "hi"},
            {"type": "text", "text": "no"},
            {"type": "tool_use", "id": "functions_Bash_0", "name": "Bash", "input": {"x": 1}}
        ]"#,
    );
}

#[test]
fn empty_unsigned_thinking_drops_but_signed_empty_stays() {
    let dropped =
        blocks(r#"[{"message": {"thinking_blocks": [{"type": "thinking", "thinking": "  "}]}}]"#);
    assert_rendered(&dropped, "[]");
    let kept = blocks(
        r#"[{"message": {"thinking_blocks": [{"type": "thinking", "thinking": "", "signature": "sig"}]}}]"#,
    );
    assert_rendered(
        &kept,
        r#"[{"type": "thinking", "thinking": "", "signature": "sig"}]"#,
    );
}

#[test]
fn null_thinking_becomes_empty_and_null_signature_stays_null() {
    let source = r#"[{"message": {"thinking_blocks": [
        {"type": "thinking", "thinking": null, "signature": "sig"},
        {"type": "thinking", "thinking": "why", "signature": null}
    ]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[
            {"type": "thinking", "thinking": "", "signature": "sig"},
            {"type": "thinking", "thinking": "why", "signature": null}
        ]"#,
    );
}

#[test]
fn missing_signature_becomes_empty_string() {
    let source = r#"[{"message": {"thinking_blocks": [{"type": "thinking", "thinking": "why"}]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "thinking", "thinking": "why", "signature": ""}]"#,
    );
}

#[test]
fn redacted_null_data_becomes_empty() {
    let source =
        r#"[{"message": {"thinking_blocks": [{"type": "redacted_thinking", "data": null}]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "redacted_thinking", "data": ""}]"#,
    );
}

#[test]
fn reasoning_falls_back_only_without_nonempty_thinking_blocks() {
    let fallback = blocks(r#"[{"message": {"reasoning_content": "why", "thinking_blocks": []}}]"#);
    assert_rendered(
        &fallback,
        r#"[{"type": "thinking", "thinking": "why", "signature": null}]"#,
    );
    let suppressed = blocks(
        r#"[{"message": {"reasoning_content": "why", "thinking_blocks": [
            {"type": "thinking", "thinking": " "}
        ]}}]"#,
    );
    assert_rendered(&suppressed, "[]");
}

#[test]
fn empty_text_needs_no_compaction() {
    let kept = blocks(r#"[{"message": {"content": ""}}]"#);
    assert_rendered(&kept, r#"[{"type": "text", "text": ""}]"#);
    let suppressed = blocks(
        r#"[{"message": {"content": "", "provider_specific_fields": {"compaction_blocks": [
            {"type": "compaction"}
        ]}}}]"#,
    );
    assert_rendered(&suppressed, r#"[{"type": "compaction"}]"#);
}

#[test]
fn null_content_is_omitted() {
    assert_rendered(&blocks(r#"[{"message": {}}]"#), "[]");
}

#[test]
fn tool_name_mapping_restores_long_name() {
    let choices = choices(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "trunc", "arguments": "{}"}}]}}]"#,
    );
    let mapping = [("trunc".to_owned(), "original_tool".to_owned())]
        .into_iter()
        .collect();
    let entries = content_blocks(&choices, Some(&mapping)).expect("translates");
    assert_rendered(
        &entries,
        r#"[{"type": "tool_use", "id": "t1", "name": "original_tool", "input": {}}]"#,
    );
}

#[test]
fn tool_id_suffix_yields_signature_but_is_stripped_from_id() {
    let source = r#"[{"message": {"tool_calls": [
        {"id": "functions.Bash:0__thought__sig", "function": {"name": "Bash", "arguments": "{}"}}
    ]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "tool_use", "id": "functions_Bash_0", "name": "Bash", "input": {},
            "provider_specific_fields": {"signature": "sig"}}]"#,
    );
}

#[test]
fn missing_tool_id_normalizes_to_fallback() {
    assert_rendered(
        &blocks(
            r#"[{"message": {"tool_calls": [{"function": {"name": "n", "arguments": "{}"}}]}}]"#,
        ),
        r#"[{"type": "tool_use", "id": "tool_use_id", "name": "n", "input": {}}]"#,
    );
}

#[test]
fn nonstream_signature_comes_from_provider_specific_fields() {
    let outer = blocks(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n", "arguments": "{}"},
            "provider_specific_fields": {"thought_signature": "outer"}}]}}]"#,
    );
    assert_rendered(
        &outer,
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {},
            "provider_specific_fields": {"signature": "outer"}}]"#,
    );
    let nested = blocks(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n", "arguments": "{}",
            "provider_specific_fields": {"thought_signature": "inner"}}}]}}]"#,
    );
    assert_rendered(
        &nested,
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {},
            "provider_specific_fields": {"signature": "inner"}}]"#,
    );
    // The outer mapping decides alone: a non-empty one without a signature hides the function's.
    let nonempty_outer = blocks(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n", "arguments": "{}",
            "provider_specific_fields": {"thought_signature": "inner"}},
            "provider_specific_fields": {"other": 1}}]}}]"#,
    );
    assert_rendered(
        &nonempty_outer,
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {}}]"#,
    );
}

#[test]
fn empty_thought_signature_adds_no_fields() {
    let source = r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n", "arguments": "{}"},
        "provider_specific_fields": {"thought_signature": ""}}]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {}}]"#,
    );
}

#[test]
fn empty_and_null_arguments_become_objects() {
    assert_rendered(
        &blocks(r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n"}}]}}]"#),
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {}}]"#,
    );
    assert_rendered(
        &blocks(
            r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "n", "arguments": " "}}]}}]"#,
        ),
        r#"[{"type": "tool_use", "id": "t1", "name": "n", "input": {}}]"#,
    );
}

#[test]
fn truncated_arguments_are_repaired() {
    let source = r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "Bash", "arguments": "{\"x\":1"}}]}}]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"x": 1}}]"#,
    );
}

#[test]
fn unparsable_arguments_name_the_tool_and_context() {
    let choices = choices(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "Bash", "arguments": "{\"x\": nope}"}}]}}]"#,
    );
    let error = content_blocks(&choices, None).expect_err("arguments cannot be repaired");
    let message = error.to_string();
    assert!(
        message.starts_with("Failed to parse tool call arguments"),
        "{message}"
    );
    assert!(message.contains("'Bash'"), "{message}");
    assert!(
        message.contains("Anthropic pass-through adapter"),
        "{message}"
    );
    assert!(message.ends_with(r#"Arguments: {"x": nope}"#), "{message}");
    assert!(error.source().is_some());
}

#[test]
fn non_object_arguments_are_refused() {
    let choices = choices(
        r#"[{"message": {"tool_calls": [{"id": "t1", "function": {"name": "Bash", "arguments": "5"}}]}}]"#,
    );
    assert!(matches!(
        content_blocks(&choices, None),
        Err(ChatOutputError::NonObjectArguments { .. })
    ));
}

#[test]
fn choices_append_in_order() {
    let source = r#"[
        {"message": {"content": "a"}},
        {"message": {"content": "b"}}
    ]"#;
    assert_rendered(
        &blocks(source),
        r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]"#,
    );
}

#[rstest]
#[case("stop", AnthropicStopReason::EndTurn)]
#[case("length", AnthropicStopReason::MaxTokens)]
#[case("tool_calls", AnthropicStopReason::ToolUse)]
#[case("content_filter", AnthropicStopReason::Refusal)]
#[case("refusal", AnthropicStopReason::Refusal)]
#[case("unknown", AnthropicStopReason::EndTurn)]
fn finish_reasons_map(#[case] source: &str, #[case] expected: AnthropicStopReason) {
    assert_eq!(anthropic_finish_reason(source), expected);
}

#[test]
fn refusal_text_forces_refusal() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{"content": "hi", "refusal": "no"}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::Refusal);
    assert_rendered(
        &translated.stop_details,
        r#"{"type": "refusal", "category": null, "explanation": "no"}"#,
    );
}

#[test]
fn provider_specific_refusal_becomes_a_text_block() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{"content": "hi",
            "provider_specific_fields": {{"refusal": "no"}}}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_rendered(
        &translated.content,
        r#"[{"type": "text", "text": "hi"}, {"type": "text", "text": "no"}]"#,
    );
}

#[test]
fn refusal_after_a_second_choice_still_counts() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [
            {{"message": {{"content": "hi"}}}},
            {{"message": {{"refusal": "no"}}}}
        ]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::Refusal);
}

#[test]
fn length_beats_refusal() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"finish_reason": "length",
            "message": {{"content": "hi", "refusal": "no"}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::MaxTokens);
    assert_rendered(&translated.stop_details, "null");
}

#[test]
fn one_compaction_block_wins_over_stop_reasons() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"finish_reason": "tool_calls",
            "message": {{"provider_specific_fields": {{"compaction_blocks": [{{"type": "compaction"}}]}}}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::Compaction);
}

#[test]
fn polyfill_compaction_block_is_inserted_first() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{"content": "hi"}}}}]}}"#
    ));
    let polyfill = polyfill_result();
    let translated = response_message(&record, None, Some(&polyfill)).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::EndTurn);
    assert_rendered(
        &translated.content,
        r#"[{"type": "compaction", "content": "summary", "signature": "cs"},
            {"type": "text", "text": "hi"}]"#,
    );
}

#[test]
fn a_polyfill_block_alone_sets_the_compaction_stop_reason() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"message": {{}}}}]}}"#
    ));
    let polyfill = PolyfillResult {
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: None,
            signature: None,
        }),
        ..PolyfillResult::default()
    };
    let translated = response_message(&record, None, Some(&polyfill)).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::Compaction);
}

#[test]
fn empty_choices_translate_to_an_empty_message() {
    let record = response(&format!(r#"{{"id": "{RESPONSE_ID}"}}"#));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::EndTurn);
    assert_rendered(
        &translated,
        r#"{"id": "chatcmpl-b966", "type": "message", "role": "assistant",
            "model": "unknown-model", "stop_sequence": null,
            "usage": {"input_tokens": 0, "output_tokens": 0},
            "content": [], "stop_reason": "end_turn", "stop_details": null}"#,
    );
}

#[test]
fn model_is_preserved_or_unknown() {
    let kept = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "model": "claude", "usage": {{"prompt_tokens": 1}}}}"#
    ));
    assert_eq!(
        response_message(&kept, None, None)
            .expect("translates")
            .model,
        "claude"
    );
    let blank = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "model": "", "usage": {{}}}}"#
    ));
    assert_eq!(
        response_message(&blank, None, None)
            .expect("translates")
            .model,
        "unknown-model"
    );
}

#[test]
fn content_filter_without_refusal_text_still_reports_refusal_details() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "choices": [{{"finish_reason": "content_filter", "message": {{}}}}]}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_eq!(translated.stop_reason, AnthropicStopReason::Refusal);
    assert_rendered(
        &translated.stop_details,
        r#"{"type": "refusal", "category": null, "explanation": null}"#,
    );
}

#[test]
fn response_ids_and_nulls_are_preserved() {
    let record = response(&format!(
        r#"{{"id": "{RESPONSE_ID}", "model": "m", "choices": [{{"message": {{"content": "hi"}}}}],
            "usage": {{"prompt_tokens": 3, "completion_tokens": 2}}}}"#
    ));
    let translated = response_message(&record, None, None).expect("translates");
    assert_rendered(
        &translated,
        r#"{"id": "chatcmpl-b966", "type": "message", "role": "assistant", "model": "m",
            "stop_sequence": null,
            "usage": {"input_tokens": 3, "output_tokens": 2},
            "content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn",
            "stop_details": null}"#,
    );
}
