#![cfg(test)]

use betterlite_openai::chat::response::{ChatChoice, ChatResponse};

use crate::api::skills::hook::response::{
    ContentBlock, GeneratedFilesAttribute, HookResponse, ObjectResponse,
};
use crate::api::skills::hook::tool_calls::{ExtractedToolCall, extract_tool_calls};

const OPENAI_TOOL_CALLS: &str = r#"[{"id":"call_1","type":"function","function":{"name":"litellm_code_execution","arguments":"{\"code\":\"from_openai\"}"}}]"#;

fn choices_with_tool_calls() -> Vec<ChatChoice> {
    let body = format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":{OPENAI_TOOL_CALLS}}},"finish_reason":"tool_calls"}}]}}"#
    );
    let parsed: ChatResponse = serde_json::from_str(&body).unwrap();
    parsed.choices
}

fn block(kind: &str, input: &str) -> ContentBlock {
    ContentBlock {
        kind: kind.to_owned(),
        id: Some("toolu_1".to_owned()),
        name: Some("litellm_skill_gif".to_owned()),
        input: serde_json::from_str(input).unwrap(),
    }
}

fn anthropic_response(content: Vec<ContentBlock>) -> HookResponse {
    HookResponse::Object(ObjectResponse {
        content,
        choices: choices_with_tool_calls(),
        generated_files: GeneratedFilesAttribute::Rejected,
        model_extra: None,
    })
}

fn expected_tool_call(id: &str, name: &str, input: &str) -> ExtractedToolCall {
    ExtractedToolCall {
        id: Some(id.to_owned()),
        name: Some(name.to_owned()),
        input: serde_json::from_str(input).unwrap(),
    }
}

#[test]
fn tool_call_extraction_preference_serves_anthropic_blocks_before_openai_choices() {
    let response = anthropic_response(vec![block("tool_use", r#"{"from": "anthropic"}"#)]);
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(
            "toolu_1",
            "litellm_skill_gif",
            r#"{"from": "anthropic"}"#
        )]
    );
}

#[test]
fn tool_call_extraction_preference_falls_back_to_openai_without_anthropic_tool_uses() {
    let response = anthropic_response(vec![block("text", r#"{"text": "no tool use here"}"#)]);
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(
            "call_1",
            "litellm_code_execution",
            r#"{"code": "from_openai"}"#
        )]
    );
}

#[test]
fn tool_call_extraction_preference_never_reads_choices_off_a_dict() {
    let body = format!(
        r#"{{"content":[{{"type":"text","text":"hi"}}],"choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":{OPENAI_TOOL_CALLS}}},"finish_reason":"tool_calls"}}]}}"#
    );
    let fields = serde_json::from_str(&body).unwrap();
    assert_eq!(
        extract_tool_calls(&HookResponse::Dict(fields)).unwrap(),
        Vec::new()
    );
}
