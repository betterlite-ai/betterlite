#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::error::HostedVllmChatError;
use super::messages::transform_messages;
use super::{ChatConfig, REASONING_EFFORT, THINKING, provider_info, supported_params};
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::local_servers::ProviderEnvironment;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("test fixture must be valid JSON")
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn config() -> ChatConfig {
    ChatConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        },
        thresholds: ThinkingBudgetThresholds::default(),
    }
}

fn params(value: Value) -> Object {
    let Value::Object(fields) = value else {
        panic!("test parameters must be an object");
    };
    fields
}

fn messages(value: Value) -> Vec<ChatMessage> {
    serde_json::from_value(value).expect("test messages must decode")
}

fn mapped(input: Value) -> Object {
    let mut optional = Object::new();
    config()
        .map_openai_params("my-model", &params(input), &mut optional, &catalog())
        .expect("mapping must succeed");
    optional
}

fn map_error(input: Value) -> HostedVllmChatError {
    let mut optional = Object::new();
    config()
        .map_openai_params("my-model", &params(input), &mut optional, &catalog())
        .expect_err("mapping must fail")
}

fn transformed(value: Value) -> Value {
    let messages = transform_messages("my-model", &messages(value), &RefuseDownloads)
        .expect("the messages must transform");
    serde_json::to_value(messages).expect("the messages must serialize")
}

fn transform_error(value: Value) -> HostedVllmChatError {
    transform_messages("my-model", &messages(value), &RefuseDownloads)
        .expect_err("the messages must not transform")
}

#[test]
fn supported_params_end_with_reasoning_effort_and_thinking() {
    let params = supported_params("my-model", &catalog());
    assert_eq!(params.len(), 33);
    assert_eq!(params.get(31), Some(&REASONING_EFFORT));
    assert_eq!(params.last(), Some(&THINKING));
}

#[test]
fn supported_params_keep_base_rules() {
    let mut known = catalog();
    known.chat_models.insert(String::from("my-model"));
    assert!(!supported_params("gpt-4", &known).contains(&"response_format"));
    assert!(supported_params("my-model", &known).contains(&"response_format"));
    assert!(supported_params("my-model", &known).contains(&"user"));
    assert!(!supported_params("my-model", &catalog()).contains(&"user"));
}

#[test]
fn map_copies_supported_and_drops_unknown_params() {
    let optional = mapped(fixture(
        r#"{
            "temperature": 0.5,
            "tools": [{"type": "function", "function": {"name": "f", "parameters": {}}}],
            "unknown_parameter": 7
        }"#,
    ));
    assert_eq!(optional.get("temperature"), Some(&Value::from(0.5_f64)));
    assert!(optional.contains_key("tools"));
    assert!(!optional.contains_key("unknown_parameter"));
}

#[test]
fn map_removes_null_tools() {
    let optional = mapped(fixture(r#"{"tools": null, "temperature": 1}"#));
    assert!(!optional.contains_key("tools"));
}

#[rstest]
#[case(0, "minimal")]
#[case(1023, "minimal")]
#[case(1024, "low")]
#[case(2047, "low")]
#[case(2048, "medium")]
#[case(4095, "medium")]
#[case(4096, "high")]
#[case(100_000, "high")]
fn enabled_thinking_sets_reasoning_effort_from_budget(#[case] budget: i64, #[case] effort: &str) {
    let optional = mapped(fixture(&format!(
        r#"{{"thinking": {{"type": "enabled", "budget_tokens": {budget}}}}}"#
    )));
    assert_eq!(optional.get(REASONING_EFFORT), Some(&json!(effort)));
}

#[test]
fn thinking_without_budget_uses_minimal() {
    let optional = mapped(fixture(r#"{"thinking": {"type": "enabled"}}"#));
    assert_eq!(optional.get(REASONING_EFFORT), Some(&json!("minimal")));
}

#[test]
fn thinking_with_null_budget_uses_minimal() {
    let optional = mapped(fixture(
        r#"{"thinking": {"type": "enabled", "budget_tokens": null}}"#,
    ));
    assert_eq!(optional.get(REASONING_EFFORT), Some(&json!("minimal")));
}

#[test]
fn explicit_reasoning_effort_wins_over_thinking() {
    let optional = mapped(fixture(
        r#"{
            "reasoning_effort": "high",
            "thinking": {"type": "enabled", "budget_tokens": 0}
        }"#,
    ));
    assert_eq!(optional.get(REASONING_EFFORT), Some(&json!("high")));
}

#[test]
fn disabled_thinking_adds_no_reasoning_effort() {
    let optional = mapped(fixture(
        r#"{"thinking": {"type": "disabled", "budget_tokens": 4096}}"#,
    ));
    assert!(!optional.contains_key(REASONING_EFFORT));
    assert!(!optional.contains_key(THINKING));
}

#[test]
fn non_object_thinking_adds_no_reasoning_effort() {
    let optional = mapped(fixture(r#"{"thinking": "enabled"}"#));
    assert!(!optional.contains_key(REASONING_EFFORT));
    assert!(!optional.contains_key(THINKING));
}

#[test]
fn non_integer_thinking_budget_is_an_error() {
    let error = map_error(fixture(
        r#"{"thinking": {"type": "enabled", "budget_tokens": "many"}}"#,
    ));
    assert!(matches!(error, HostedVllmChatError::ThinkingBudget));
}

#[test]
fn custom_tool_becomes_a_function_tool() {
    let optional = mapped(fixture(
        r#"{
            "tools": [{
                "type": "custom",
                "custom": {
                    "name": "lookup",
                    "description": "looks things up",
                    "input_schema": {"type": "object", "properties": {"q": {"type": "string"}}}
                }
            }]
        }"#,
    ));
    assert_eq!(
        optional.get("tools"),
        Some(&fixture(
            r#"[{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "looks things up",
                    "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}
                }
            }]"#
        ))
    );
}

#[test]
fn custom_tool_falls_back_to_the_tool_level_fields() {
    let optional = mapped(fixture(
        r#"{
            "tools": [{
                "type": "custom",
                "name": "outer",
                "description": "outer description",
                "input_schema": {"type": "object", "properties": {}}
            }]
        }"#,
    ));
    assert_eq!(
        optional.get("tools"),
        Some(&fixture(
            r#"[{
                "type": "function",
                "function": {
                    "name": "outer",
                    "description": "outer description",
                    "parameters": {"type": "object", "properties": {}}
                }
            }]"#
        ))
    );
}

#[test]
fn custom_tool_without_a_name_gets_an_indexed_name() {
    let optional = mapped(fixture(
        r#"{
            "tools": [
                {"type": "function", "function": {"name": "kept", "parameters": {}}},
                {"type": "custom"}
            ]
        }"#,
    ));
    assert_eq!(
        optional.get("tools"),
        Some(&fixture(
            r#"[
                {"type": "function", "function": {"name": "kept", "parameters": {}}},
                {
                    "type": "function",
                    "function": {
                        "name": "custom_tool_1",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "input": {
                                    "type": "string",
                                    "description": "Raw tool input payload."
                                }
                            },
                            "required": ["input"]
                        }
                    }
                }
            ]"#
        ))
    );
}

#[test]
fn custom_tool_without_an_input_schema_gets_the_default_schema() {
    let optional = mapped(fixture(
        r#"{"tools": [{"type": "custom", "custom": {"name": "n"}}]}"#,
    ));
    let tools = optional
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools");
    let function = tools
        .first()
        .and_then(|tool| tool.get("function"))
        .expect("function");
    assert_eq!(
        function.get("parameters"),
        Some(&fixture(
            r#"{
                "type": "object",
                "properties": {
                    "input": {"type": "string", "description": "Raw tool input payload."}
                },
                "required": ["input"]
            }"#
        ))
    );
}

#[test]
fn custom_tool_drops_an_empty_description() {
    let optional = mapped(fixture(
        r#"{"tools": [{"type": "custom", "custom": {"name": "n", "description": ""}}]}"#,
    ));
    let function = optional
        .get("tools")
        .and_then(|tools| tools.get(0))
        .and_then(|tool| tool.get("function"))
        .expect("function");
    assert!(
        !function
            .as_object()
            .expect("object")
            .contains_key("description")
    );
}

#[test]
fn custom_tool_keeps_an_empty_tool_level_description() {
    let optional = mapped(fixture(
        r#"{"tools": [{"type": "custom", "name": "n", "description": ""}]}"#,
    ));
    let function = optional
        .get("tools")
        .and_then(|tools| tools.get(0))
        .and_then(|tool| tool.get("function"))
        .expect("function");
    assert_eq!(function.get("description"), Some(&json!("")));
}

#[test]
fn custom_tool_keeps_an_empty_tool_level_input_schema() {
    let optional = mapped(fixture(
        r#"{"tools": [{"type": "custom", "name": "n", "input_schema": {}}]}"#,
    ));
    let function = optional
        .get("tools")
        .and_then(|tools| tools.get(0))
        .and_then(|tool| tool.get("function"))
        .expect("function");
    assert_eq!(function.get("parameters"), Some(&fixture("{}")));
}

#[test]
fn empty_custom_fields_fall_back_to_the_tool_level_fields() {
    let optional = mapped(fixture(
        r#"{
            "tools": [{
                "type": "custom",
                "custom": {"name": "", "description": "", "input_schema": {}},
                "name": "outer",
                "description": "outer description",
                "input_schema": {"type": "object", "properties": {}}
            }]
        }"#,
    ));
    let function = optional
        .get("tools")
        .and_then(|tools| tools.get(0))
        .and_then(|tool| tool.get("function"))
        .expect("function");
    assert_eq!(function.get("name"), Some(&json!("outer")));
    assert_eq!(
        function.get("description"),
        Some(&json!("outer description"))
    );
    assert_eq!(
        function.get("parameters"),
        Some(&fixture(r#"{"type": "object", "properties": {}}"#))
    );
}

#[test]
fn non_custom_tools_are_kept_and_cleaned() {
    let optional = mapped(fixture(
        r#"{
            "tools": [{
                "type": "function",
                "function": {
                    "name": "f",
                    "strict": true,
                    "parameters": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {}
                    }
                }
            }]
        }"#,
    ));
    assert_eq!(
        optional.get("tools"),
        Some(&fixture(
            r#"[{
                "type": "function",
                "function": {
                    "name": "f",
                    "parameters": {"type": "object", "properties": {}}
                }
            }]"#
        ))
    );
}

#[test]
fn a_tool_that_is_not_an_object_is_an_error() {
    let error = map_error(fixture(r#"{"tools": ["f"]}"#));
    assert!(matches!(
        error,
        HostedVllmChatError::ToolNotObject { index: 0 }
    ));
}

#[test]
fn a_custom_field_that_is_not_an_object_is_an_error() {
    let error = map_error(fixture(r#"{"tools": [{"type": "custom", "custom": 3}]}"#));
    assert!(matches!(error, HostedVllmChatError::CustomToolObject));
}

#[test]
fn a_tool_name_that_is_not_a_string_is_an_error() {
    let error = map_error(fixture(
        r#"{"tools": [{"type": "custom", "custom": {"name": 3}}]}"#,
    ));
    assert!(matches!(
        error,
        HostedVllmChatError::CustomToolString { field: "name" }
    ));
}

#[test]
fn an_input_schema_that_is_not_an_object_is_an_error() {
    let error = map_error(fixture(
        r#"{"tools": [{"type": "custom", "custom": {"input_schema": "schema"}}]}"#,
    ));
    assert!(matches!(error, HostedVllmChatError::CustomToolSchema));
}

#[test]
fn a_video_file_data_item_becomes_a_video_url() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_data": "data:video/mp4;base64,AAAA", "format": "video/mp4"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{
                "type": "video_url",
                "video_url": {"url": "data:video/mp4;base64,AAAA"}
            }]"#
        ))
    );
}

#[test]
fn a_video_file_data_item_without_a_format_becomes_a_video_url() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_data": "data:video/mp4;base64,AAAA"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{
                "type": "video_url",
                "video_url": {"url": "data:video/mp4;base64,AAAA"}
            }]"#
        ))
    );
}

#[test]
fn a_file_id_wins_over_file_data_for_the_video_url() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_id": "clip.mp4", "file_data": "data:video/mp4;base64,AAAA"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{"type": "video_url", "video_url": {"url": "clip.mp4"}}]"#
        ))
    );
}

#[test]
fn a_video_file_id_item_becomes_a_video_url() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_id": "https://host/clip.mp4"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{"type": "video_url", "video_url": {"url": "https://host/clip.mp4"}}]"#
        ))
    );
}

#[test]
fn a_video_format_without_a_source_is_an_error() {
    let error = transform_error(fixture(
        r#"[{
            "role": "user",
            "content": [{"type": "file", "file": {"format": "video/mp4"}}]
        }]"#,
    ));
    assert!(matches!(error, HostedVllmChatError::MissingVideoSource));
}

#[test]
fn file_data_decides_before_a_file_id() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_data": "data:image/png;base64,AAAA", "file_id": "clip.mp4"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{
                "type": "file",
                "file": {
                    "file_data": "data:image/png;base64,AAAA",
                    "file_id": "clip.mp4",
                    "filename": "my_file.pdf"
                }
            }]"#
        ))
    );
}

#[test]
fn a_file_without_a_video_source_stays_a_file() {
    let output = transformed(fixture(
        r#"[{
            "role": "user",
            "content": [{
                "type": "file",
                "file": {"file_data": "data:image/png;base64,AAAA", "format": "image/png"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[{
                "type": "file",
                "file": {"file_data": "data:image/png;base64,AAAA", "filename": "my_file.pdf"}
            }]"#
        ))
    );
}

#[test]
fn assistant_text_blocks_join_into_a_string() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&json!("a\nb"))
    );
}

#[test]
fn assistant_null_text_contributes_an_empty_string() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{"type": "text", "text": "a"}, {"type": "text", "text": null}]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&json!("a\n"))
    );
}

#[test]
fn assistant_tool_use_blocks_become_tool_calls() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [
                {"type": "text", "text": "one"},
                {"type": "tool_use", "id": "call_1", "name": "lookup", "input": "{\"q\":\"x\"}"}
            ]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("tool_calls")),
        Some(&fixture(
            r#"[{
                "id": "call_1",
                "type": "function",
                "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}
            }]"#
        ))
    );
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&json!("one"))
    );
}

#[test]
fn assistant_tool_use_with_an_object_input_encodes_it_as_json() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "call_1",
                "name": "lookup",
                "input": {"q": "x"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("tool_calls")),
        Some(&fixture(
            r#"[{
                "id": "call_1",
                "type": "function",
                "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}
            }]"#
        ))
    );
}

#[test]
fn assistant_tool_use_without_input_sends_an_empty_object() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "call_1", "name": "lookup"}]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("tool_calls")),
        Some(&fixture(
            r#"[{
                "id": "call_1",
                "type": "function",
                "function": {"name": "lookup", "arguments": "{}"}
            }]"#
        ))
    );
}

#[test]
fn assistant_tool_calls_dedupe_by_existing_ids() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "call_1", "name": "again", "input": "{}"}],
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "first", "arguments": "{}"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("tool_calls")),
        Some(&fixture(
            r#"[{
                "id": "call_1",
                "type": "function",
                "function": {"name": "first", "arguments": "{}"}
            }]"#
        ))
    );
}

#[test]
fn assistant_tool_calls_append_new_calls_to_existing_ones() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "call_2",
                "name": "lookup",
                "input": "{\"q\":\"x\"}"
            }],
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "first", "arguments": "{}"}
            }]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("tool_calls")),
        Some(&fixture(
            r#"[
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "first", "arguments": "{}"}
                },
                {
                    "id": "call_2",
                    "type": "function",
                    "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}
                }
            ]"#
        ))
    );
}

#[test]
fn assistant_structured_blocks_keep_the_block_content() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": [
                {"type": "text", "text": "one"},
                {"type": "image_url", "image_url": {"url": "https://host/i.png"}}
            ]
        }]"#,
    ));
    assert_eq!(
        output.get(0).and_then(|message| message.get("content")),
        Some(&fixture(
            r#"[
                {"type": "text", "text": "one"},
                {"type": "image_url", "image_url": {"url": "https://host/i.png"}}
            ]"#
        ))
    );
}

#[test]
fn assistant_thinking_fields_are_removed() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": "hello",
            "thinking_blocks": [{"type": "thinking", "thinking": "t"}],
            "reasoning_content": {"text": "not a string"}
        }]"#,
    ));
    let message = output.get(0).and_then(Value::as_object).expect("message");
    assert!(!message.contains_key("thinking_blocks"));
    assert!(!message.contains_key("reasoning_content"));
}

#[test]
fn assistant_string_reasoning_content_is_kept() {
    let output = transformed(fixture(
        r#"[{
            "role": "assistant",
            "content": "hello",
            "reasoning_content": "kept"
        }]"#,
    ));
    assert_eq!(
        output
            .get(0)
            .and_then(|message| message.get("reasoning_content")),
        Some(&json!("kept"))
    );
}

#[test]
fn a_tool_use_without_a_string_id_is_an_error() {
    let error = transform_error(fixture(
        r#"[{
            "role": "assistant",
            "content": [{"type": "tool_use", "name": "lookup"}]
        }]"#,
    ));
    assert!(matches!(
        error,
        HostedVllmChatError::PartString {
            part: "tool_use",
            field: "id"
        }
    ));
}

#[test]
fn a_file_field_that_is_not_an_object_is_an_error() {
    let error = transform_error(fixture(
        r#"[{
            "role": "user",
            "content": [{"type": "file", "file": "nope"}]
        }]"#,
    ));
    assert!(matches!(error, HostedVllmChatError::FileObject));
}

#[test]
fn provider_info_prefers_explicit_arguments() {
    let environment = ProviderEnvironment {
        api_base: Some(String::from("http://env")),
        api_key: Some(SecretString::from(String::from("env-key"))),
    };
    let key = SecretString::from(String::from("argument-key"));
    let info = provider_info(Some("http://argument"), Some(&key), &environment);
    assert_eq!(info.api_base.as_deref(), Some("http://argument"));
    assert_eq!(info.api_key.expose_secret(), "argument-key");
}

#[test]
fn provider_info_reads_the_environment_and_defaults_the_key() {
    let environment = ProviderEnvironment {
        api_base: Some(String::from("http://env")),
        api_key: None,
    };
    let info = provider_info(None, None, &environment);
    assert_eq!(info.api_base.as_deref(), Some("http://env"));
    assert_eq!(info.api_key.expose_secret(), "fake-api-key");
}

#[test]
fn provider_info_falls_through_empty_arguments() {
    let environment = ProviderEnvironment {
        api_base: Some(String::from("http://env")),
        api_key: Some(SecretString::from(String::from("env-key"))),
    };
    let key = SecretString::from(String::new());
    let info = provider_info(Some(""), Some(&key), &environment);
    assert_eq!(info.api_base.as_deref(), Some("http://env"));
    assert_eq!(info.api_key.expose_secret(), "env-key");
}

#[test]
fn provider_info_without_any_base_keeps_none() {
    let info = provider_info(None, None, &ProviderEnvironment::default());
    assert_eq!(info.api_base, None);
    assert_eq!(info.api_key.expose_secret(), "fake-api-key");
}
