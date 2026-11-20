#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use serde_json::{Value, json};

use super::error::ProcessOutputError;
use super::{apply_responses_to_output, extract_from_content_blocks, process_output_response};
use crate::guardrails::anthropic_translation::process_input::testing::{
    NO_INPUTS, ScriptedGuardrail,
};

fn value(raw: Value) -> GuardrailValue {
    serde_json::from_value(raw).expect("a GuardrailValue")
}

fn to_json(item: &GuardrailValue) -> Value {
    serde_json::to_value(item).expect("a JSON value")
}

fn blocks(raw: Value) -> Vec<Value> {
    raw.as_array().cloned().expect("an array of blocks")
}

fn answer_texts(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        ..NO_INPUTS
    }
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

#[test]
fn output_extract_texts_and_tool_calls_with_positions() {
    let content = blocks(json!([
        text_block("first"),
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}},
        {"type": "thinking", "thinking": "hmm"},
        text_block("second")
    ]));
    let extracted = extract_from_content_blocks(&content).unwrap();
    assert_eq!(extracted.texts, vec!["first", "second"]);
    assert_eq!(extracted.mappings, vec![0, 3]);
    assert_eq!(extracted.tool_calls.len(), 1);
    assert_eq!(extracted.tool_calls[0]["index"].as_u64(), Some(1));
    assert_eq!(extracted.tool_calls[0]["id"].as_str(), Some("toolu_1"));
    assert_eq!(
        extracted.tool_calls[0]["function"]["arguments"].as_str(),
        Some("{\"a\": 1}")
    );
}

#[test]
fn output_extract_skips_empty_text_and_unknown_blocks() {
    let content = blocks(json!([
        text_block(""),
        {"type": "image", "source": {"type": "url", "url": "http://x"}},
        "junk",
        {"text": "no type"},
        text_block("kept")
    ]));
    let extracted = extract_from_content_blocks(&content).unwrap();
    assert_eq!(extracted.texts, vec!["kept"]);
    assert_eq!(extracted.mappings, vec![4]);
    assert!(extracted.tool_calls.is_empty());
}

#[test]
fn output_extract_malformed_tool_use_names_the_position() {
    let content = blocks(json!([
        text_block("a"),
        {"type": "tool_use", "name": "get", "input": {}}
    ]));
    let error = extract_from_content_blocks(&content).unwrap_err();
    assert!(matches!(
        error,
        ProcessOutputError::MalformedToolUse { content_idx: 1 }
    ));
}

#[tokio::test]
async fn output_call_sends_texts_tool_calls_and_model() {
    let mut response = value(json!({
        "model": "claude-sonnet-4-5",
        "content": [
            text_block("hello"),
            {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}
        ]
    }));
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let recording = guardrail.recording();
    assert_eq!(recording.input_type, GuardrailInputType::Response);
    assert_eq!(recording.inputs.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(recording.inputs.model, Some("claude-sonnet-4-5".to_owned()));
    assert!(recording.inputs.images.is_none());
    let sent = recording.inputs.tool_calls.expect("tool calls");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0],
        value(json!({
            "id": "toolu_1",
            "type": "function",
            "function": {"name": "get", "arguments": "{\"a\": 1}"},
            "index": 1
        }))
    );
}

#[tokio::test]
async fn output_call_skips_a_response_without_content() {
    let mut response = value(json!({"model": "claude-sonnet-4-5"}));
    let before = response.clone();
    let mut request_data = BTreeMap::new();
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        None,
        Some(&mut request_data),
    )
    .await
    .unwrap();
    assert!(!guardrail.ran());
    assert!(request_data.is_empty());
    assert_eq!(response, before);
}

#[tokio::test]
async fn output_call_skips_a_response_with_nothing_to_send() {
    let mut response = value(json!({
        "model": "claude-sonnet-4-5",
        "content": [
            {"type": "image", "source": {"type": "url", "url": "http://x"}},
            {"type": "thinking", "thinking": "hmm"}
        ]
    }));
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    assert!(!guardrail.ran());
}

#[tokio::test]
async fn output_call_adds_the_response_context() {
    let mut response = value(json!({
        "model": "claude-sonnet-4-5",
        "content": [text_block("hello")]
    }));
    let original = response.clone();
    let mut request_data = BTreeMap::from([("model".to_owned(), value(json!("kept")))]);
    let guardrail = ScriptedGuardrail::new(NO_INPUTS);
    process_output_response(
        &mut response,
        &guardrail,
        None,
        None,
        Some(&mut request_data),
    )
    .await
    .unwrap();
    let recorded = guardrail.recording().request_data;
    assert_eq!(recorded.get("response"), Some(&original));
    assert_eq!(recorded.get("model"), Some(&value(json!("kept"))));
    assert!(!recorded.contains_key("litellm_metadata"));
}

#[tokio::test]
async fn output_apply_shorter_reply_rewrites_the_prefix() {
    let mut content = blocks(json!([
        text_block("first"),
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {}},
        text_block("second")
    ]));
    apply_responses_to_output(&mut content, &["X".to_owned()], &[0, 2]).unwrap();
    assert_eq!(content[0]["text"].as_str(), Some("X"));
    assert_eq!(content[2]["text"].as_str(), Some("second"));
    assert_eq!(content[1]["type"].as_str(), Some("tool_use"));
}

#[tokio::test]
async fn output_apply_longer_reply_is_too_many_texts() {
    let mut content = blocks(json!([text_block("first"), text_block("second")]));
    let responses = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let error = apply_responses_to_output(&mut content, &responses, &[0, 1]).unwrap_err();
    assert!(matches!(
        error,
        ProcessOutputError::TooManyTexts {
            sent: 2,
            returned: 3
        }
    ));
}

#[tokio::test]
async fn output_apply_leaves_non_text_blocks_and_missing_positions_alone() {
    let mut content = blocks(json!([
        {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {}}
    ]));
    let before = content.clone();
    apply_responses_to_output(&mut content, &["x".to_owned()], &[0]).unwrap();
    assert_eq!(content, before);
    apply_responses_to_output(&mut content, &["x".to_owned()], &[7]).unwrap();
    assert_eq!(content, before);
}

#[tokio::test]
async fn output_apply_rewrites_the_response_in_place() {
    let mut response = value(json!({
        "model": "claude-sonnet-4-5",
        "content": [
            text_block("first"),
            {"type": "tool_use", "id": "toolu_1", "name": "get", "input": {}},
            text_block("second")
        ]
    }));
    let guardrail = ScriptedGuardrail::new(answer_texts(&["X", "Y"]));
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let written = to_json(&response);
    assert_eq!(written["content"][0]["text"].as_str(), Some("X"));
    assert_eq!(written["content"][2]["text"].as_str(), Some("Y"));
    assert_eq!(written["model"].as_str(), Some("claude-sonnet-4-5"));
}

#[tokio::test]
async fn output_apply_returned_tool_calls_are_never_written_back() {
    let mut response = value(json!({
        "model": "claude-sonnet-4-5",
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "get", "input": {"a": 1}}]
    }));
    let rewritten = value(json!({
        "id": "toolu_1",
        "type": "function",
        "function": {"name": "get", "arguments": "{\"a\": 2}"},
        "index": 0
    }));
    let answer = GuardrailApiInputs {
        tool_calls: Some(vec![rewritten]),
        ..NO_INPUTS
    };
    let guardrail = ScriptedGuardrail::new(answer);
    process_output_response(&mut response, &guardrail, None, None, None)
        .await
        .unwrap();
    let written = to_json(&response);
    assert_eq!(written["content"][0]["input"]["a"].as_u64(), Some(1));
}
