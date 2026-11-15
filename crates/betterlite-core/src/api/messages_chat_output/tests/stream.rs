#![cfg(test)]

use crate::api::messages_chat_output::error::ChatOutputError;
use crate::api::messages_chat_output::stream::{content_block, delta};

use super::support::{assert_rendered, choices, fixed_ids};

#[test]
fn block_selection_prefers_a_tool_call() {
    let choices = choices(
        r#"[{"delta": {"content": "hi", "tool_calls": [{"id": "t1", "function": {"name": "n"}}]}}]"#,
    );
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "tool_use", "id": "t1", "name": "n", "input": {}}"#,
    );
}

#[test]
fn block_selection_generates_an_id_and_keeps_an_empty_name() {
    let choices = choices(r#"[{"delta": {"tool_calls": [{"function": {}}]}}]"#);
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "tool_use", "id": "00000000-0000-0000-0000-000000000060", "name": "",
            "input": {}}"#,
    );
}

#[test]
fn block_selection_splits_the_thought_signature() {
    let choices = choices(
        r#"[{"delta": {"tool_calls": [{"id": "t__thought__sig", "function": {"name": "n"}}]}}]"#,
    );
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "tool_use", "id": "t", "name": "n", "input": {},
            "provider_specific_fields": {"signature": "sig"}}"#,
    );
}

#[test]
fn block_selection_opens_an_empty_text_block_for_text_or_refusal() {
    let text = choices(r#"[{"delta": {"content": "hi"}}]"#);
    assert_rendered(
        &content_block(&text, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
    let refusal = choices(r#"[{"delta": {"refusal": "no"}}]"#);
    assert_rendered(
        &content_block(&refusal, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
    let provider_refusal =
        choices(r#"[{"delta": {"provider_specific_fields": {"refusal": "no"}}}]"#);
    assert_rendered(
        &content_block(&provider_refusal, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
}

#[test]
fn block_selection_opens_a_typed_thinking_block() {
    let choices = choices(
        r#"[{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "why", "signature": "sig"}]}}]"#,
    );
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "thinking", "thinking": "why", "signature": "sig"}"#,
    );
}

#[test]
fn block_selection_falls_through_a_non_thinking_first_block() {
    let choices = choices(
        r#"[{"delta": {"thinking_blocks": [{"type": "redacted_thinking", "data": "d"}]}},
            {"delta": {"content": "hi"}}]"#,
    );
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
}

#[test]
fn block_selection_falls_through_a_null_function() {
    let choices = choices(r#"[{"delta": {"content": "hi", "tool_calls": [{"function": null}]}}]"#);
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
}

#[test]
fn block_selection_uses_reasoning_only_without_thinking_blocks() {
    let reasoning = choices(r#"[{"delta": {"reasoning_content": "why"}}]"#);
    assert_rendered(
        &content_block(&reasoning, &fixed_ids()).expect("selects a block"),
        r#"{"type": "thinking", "thinking": "", "signature": ""}"#,
    );
    let suppressed = choices(r#"[{"delta": {"reasoning_content": "why", "thinking_blocks": []}}]"#);
    assert_rendered(
        &content_block(&suppressed, &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
}

#[test]
fn block_selection_without_choices_opens_an_empty_text_block() {
    assert_rendered(
        &content_block(&[], &fixed_ids()).expect("selects a block"),
        r#"{"type": "text", "text": ""}"#,
    );
}

#[test]
fn block_selection_uses_the_next_choice() {
    let choices = choices(
        r#"[{"delta": {}}, {"delta": {"tool_calls": [{"id": "t1", "function": {"name": "n"}}]}}]"#,
    );
    assert_rendered(
        &content_block(&choices, &fixed_ids()).expect("selects a block"),
        r#"{"type": "tool_use", "id": "t1", "name": "n", "input": {}}"#,
    );
}

#[test]
fn delta_combines_tool_arguments_and_resets_per_choice() {
    let combined = choices(
        r#"[{"delta": {"tool_calls": [
            {"function": {"arguments": "{\"a\":"}},
            {"function": {"arguments": "1}"}},
            {"function": {"name": "ignored"}}
        ]}}]"#,
    );
    assert_rendered(
        &delta(&combined).expect("projects a delta"),
        r#"{"type": "input_json_delta", "partial_json": "{\"a\":1}"}"#,
    );
    let reset = choices(
        r#"[{"delta": {"tool_calls": [{"function": {"arguments": "first"}}]}},
            {"delta": {"tool_calls": [{"function": {"arguments": "second"}}, {"function": {}}]}}]"#,
    );
    assert_rendered(
        &delta(&reset).expect("projects a delta"),
        r#"{"type": "input_json_delta", "partial_json": "second"}"#,
    );
}

#[test]
fn delta_without_arguments_still_emits_an_empty_partial_json() {
    let choices = choices(r#"[{"delta": {"tool_calls": [{"function": {"name": "n"}}]}}]"#);
    assert_rendered(
        &delta(&choices).expect("projects a delta"),
        r#"{"type": "input_json_delta", "partial_json": ""}"#,
    );
}

#[test]
fn delta_prefers_signature_then_thinking_then_text() {
    let signed = choices(
        r#"[{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "why", "signature": "sig"}]}}]"#,
    );
    assert_rendered(
        &delta(&signed).expect("projects a delta"),
        r#"{"type": "signature_delta", "signature": "sig"}"#,
    );
    let thinking =
        choices(r#"[{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": "why"}]}}]"#);
    assert_rendered(
        &delta(&thinking).expect("projects a delta"),
        r#"{"type": "thinking_delta", "thinking": "why"}"#,
    );
    let text = choices(r#"[{"delta": {"content": "a"}}, {"delta": {"content": "b"}}]"#);
    assert_rendered(
        &delta(&text).expect("projects a delta"),
        r#"{"type": "text_delta", "text": "ab"}"#,
    );
}

#[test]
fn delta_appends_refusals_after_text() {
    let choices = choices(
        r#"[{"delta": {"content": "a", "refusal": "x"}}, {"delta": {"content": "b", "refusal": "y"}}]"#,
    );
    assert_rendered(
        &delta(&choices).expect("projects a delta"),
        r#"{"type": "text_delta", "text": "abxy"}"#,
    );
}

#[test]
fn delta_accumulates_every_thinking_block() {
    let choices = choices(
        r#"[{"delta": {"thinking_blocks": [
            {"type": "thinking", "thinking": "a", "signature": "s"},
            {"type": "redacted_thinking", "data": "d"},
            {"type": "thinking", "thinking": "b", "signature": "t"}
        ]}}]"#,
    );
    assert_rendered(
        &delta(&choices).expect("projects a delta"),
        r#"{"type": "signature_delta", "signature": "st"}"#,
    );
}

#[test]
fn delta_reads_nullable_reasoning_as_empty() {
    let choices = choices(
        r#"[{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": null, "signature": null}]}}]"#,
    );
    assert_rendered(
        &delta(&choices).expect("projects a delta"),
        r#"{"type": "text_delta", "text": ""}"#,
    );
}

#[test]
fn delta_skips_reasoning_beside_thinking_blocks() {
    let choices = choices(r#"[{"delta": {"reasoning_content": "why", "thinking_blocks": []}}]"#);
    assert_rendered(
        &delta(&choices).expect("projects a delta"),
        r#"{"type": "text_delta", "text": ""}"#,
    );
}

#[test]
fn delta_refuses_a_nonstring_thinking_field() {
    let choices =
        choices(r#"[{"delta": {"thinking_blocks": [{"type": "thinking", "thinking": 7}]}}]"#);
    assert!(matches!(
        delta(&choices),
        Err(ChatOutputError::NonTextThinkingField { field: "thinking" })
    ));
}

#[test]
fn delta_without_choices_emits_empty_text() {
    assert_rendered(
        &delta(&[]).expect("projects a delta"),
        r#"{"type": "text_delta", "text": ""}"#,
    );
}
