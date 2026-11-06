#![cfg(test)]

use betterlite_openai::chat::response::ChatResponse;

use crate::api::skills::hook::response::{GeneratedFilesAttribute, HookResponse, ObjectResponse};
use crate::api::skills::hook::tool_calls::{ExtractedToolCall, extract_tool_calls};

fn response_with_choices(choices: &str) -> HookResponse {
    let body = format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":{choices}}}"#
    );
    let parsed: ChatResponse = serde_json::from_str(&body).unwrap();
    HookResponse::Object(ObjectResponse {
        content: Vec::new(),
        choices: parsed.choices,
        generated_files: GeneratedFilesAttribute::Rejected,
        model_extra: None,
    })
}

fn response_with_tool_calls(tool_calls: &str) -> HookResponse {
    let choices = format!(
        r#"[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":{tool_calls}}},"finish_reason":"tool_calls"}}]"#
    );
    response_with_choices(&choices)
}

fn choice_with_tool_calls(tool_calls: &str) -> String {
    format!(
        r#"{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":{tool_calls}}},"finish_reason":"tool_calls"}}"#
    )
}

fn expected_tool_call(id: &str, name: &str, input: &str) -> ExtractedToolCall {
    ExtractedToolCall {
        id: Some(id.to_owned()),
        name: Some(name.to_owned()),
        input: serde_json::from_str(input).unwrap(),
    }
}

#[test]
fn tool_call_extraction_openai_reads_the_first_choice_tool_calls() {
    let response = response_with_tool_calls(
        r#"[{"id":"call_1","type":"function","function":{"name":"litellm_code_execution","arguments":"{\"code\":\"print(1)\"}"}}]"#,
    );
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call(
            "call_1",
            "litellm_code_execution",
            r#"{"code": "print(1)"}"#
        )]
    );
}

#[test]
fn tool_call_extraction_openai_reads_every_tool_call_of_the_first_choice() {
    let response = response_with_tool_calls(
        r#"[
            {"id":"call_1","type":"function","function":{"name":"litellm_code_execution","arguments":"{\"code\":\"a\"}"}},
            {"id":"call_2","type":"function","function":{"name":"litellm_skill_gif","arguments":"{\"style\":\"flat\"}"}}
        ]"#,
    );
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![
            expected_tool_call("call_1", "litellm_code_execution", r#"{"code": "a"}"#),
            expected_tool_call("call_2", "litellm_skill_gif", r#"{"style": "flat"}"#),
        ]
    );
}

#[test]
fn tool_call_extraction_openai_takes_an_empty_input_for_empty_arguments() {
    let response = response_with_tool_calls(
        r#"[{"id":"call_1","type":"function","function":{"name":"litellm_code_execution","arguments":""}}]"#,
    );
    assert_eq!(
        extract_tool_calls(&response).unwrap(),
        vec![expected_tool_call("call_1", "litellm_code_execution", "{}")]
    );
}

#[test]
fn tool_call_extraction_openai_ignores_later_choices() {
    let second = choice_with_tool_calls(
        r#"[{"id":"call_2","type":"function","function":{"name":"litellm_code_execution","arguments":"{\"code\":\"b\"}"}}]"#,
    );
    let response = response_with_choices(&format!(
        r#"[{{"index":0,"message":{{"role":"assistant","content":"done"}},"finish_reason":"stop"}},{second}]"#
    ));
    assert_eq!(extract_tool_calls(&response).unwrap(), Vec::new());
}

#[test]
fn tool_call_extraction_openai_without_choices_is_empty() {
    assert_eq!(
        extract_tool_calls(&response_with_choices("[]")).unwrap(),
        Vec::new()
    );
}
