#![cfg(test)]

use crate::api::skills::hook::response::{
    ContentBlock, GeneratedFilesAttribute, HookResponse, ObjectResponse,
};
use crate::api::skills::hook::tool_calls::{ExtractedToolCall, extract_tool_calls};

fn dict_response(body: &str) -> HookResponse {
    let fields = serde_json::from_str(body).unwrap();
    HookResponse::Dict(fields)
}

fn object_response(content: Vec<ContentBlock>) -> HookResponse {
    HookResponse::Object(ObjectResponse {
        content,
        choices: Vec::new(),
        generated_files: GeneratedFilesAttribute::Rejected,
        model_extra: None,
    })
}

fn expected_tool_call(id: Option<&str>, name: Option<&str>, input: &str) -> ExtractedToolCall {
    ExtractedToolCall {
        id: id.map(str::to_owned),
        name: name.map(str::to_owned),
        input: serde_json::from_str(input).unwrap(),
    }
}

fn block(kind: &str, id: Option<&str>, name: Option<&str>, input: &str) -> ContentBlock {
    ContentBlock {
        kind: kind.to_owned(),
        id: id.map(str::to_owned),
        name: name.map(str::to_owned),
        input: serde_json::from_str(input).unwrap(),
    }
}

#[test]
fn tool_call_extraction_anthropic_dict_blocks_take_tool_use_entries() {
    let response = dict_response(
        r#"{
            "content": [
                {"type": "text", "text": "thinking"},
                {"type": "tool_use", "id": "toolu_1", "name": "litellm_code_execution", "input": {"code": "print(1)"}},
                {"type": "tool_use", "id": "toolu_2", "name": "litellm_skill_gif", "input": {"style": "flat"}}
            ]
        }"#,
    );
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![
            expected_tool_call(
                Some("toolu_1"),
                Some("litellm_code_execution"),
                r#"{"code": "print(1)"}"#
            ),
            expected_tool_call(
                Some("toolu_2"),
                Some("litellm_skill_gif"),
                r#"{"style": "flat"}"#
            ),
        ]
    );
}

#[test]
fn tool_call_extraction_anthropic_dict_block_without_input_takes_an_empty_object() {
    let response = dict_response(
        r#"{"content": [{"type": "tool_use", "id": "toolu_1", "name": "litellm_code_execution"}]}"#,
    );
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(
            Some("toolu_1"),
            Some("litellm_code_execution"),
            "{}"
        )]
    );
}

#[test]
fn tool_call_extraction_anthropic_dict_blocks_skip_other_types_and_shapes() {
    let response = dict_response(r#"{"content": ["a string", 7, {"type": "text", "text": "hi"}]}"#);
    assert_eq!(extract_tool_calls(&response).unwrap(), Vec::new());
}

#[test]
fn tool_call_extraction_anthropic_dict_without_content_is_empty() {
    let response = dict_response(r#"{"stop_reason": "end_turn"}"#);
    assert_eq!(extract_tool_calls(&response).unwrap(), Vec::new());
}

#[test]
fn tool_call_extraction_anthropic_object_blocks_take_tool_use_entries() {
    let response = object_response(vec![
        block("text", None, None, r#"{"text": "thinking"}"#),
        block(
            "tool_use",
            Some("toolu_9"),
            Some("litellm_skill_gif"),
            r#"{"steps": 3}"#,
        ),
    ]);
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(
            Some("toolu_9"),
            Some("litellm_skill_gif"),
            r#"{"steps": 3}"#
        )]
    );
}

#[test]
fn tool_call_extraction_anthropic_object_block_without_input_takes_an_empty_object() {
    let response = object_response(vec![block("tool_use", None, None, "null")]);
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(None, None, "{}")]
    );
}
