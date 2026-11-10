use betterlite_openai::chat::{ChatRequest, RequestError};
use rstest::rstest;

use crate::oci_chat_wire::generic::request::{adapt_generic_messages, adapt_generic_tools};
use crate::oci_chat_wire::types::oci::{OciChatRequestPayload, OciMessage, OciRole, OciVendor};

use super::support::{assert_json, messages, tools};

#[test]
fn vendor_api_format_uses_generic_for_any_non_cohere_vendor() {
    assert_eq!(OciVendor::Cohere.api_format(), "COHERE");
    assert_eq!(OciVendor::Generic.api_format(), "GENERIC");
}

#[test]
fn a_tool_call_without_arguments_fails_the_shared_request_parse() {
    // documented in docs/deviations/B5942.yaml: LiteLLM defaults the absent value
    let error = ChatRequest::parse(
        br#"{"model": "m", "messages": [{"role": "assistant", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f"}}
        ]}]}"#,
    )
    .unwrap_err();
    assert!(matches!(error, RequestError::Body { .. }));
}

#[test]
fn generic_messages_convert_roles_and_text() {
    let adapted = adapt_generic_messages(&messages(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "there"}
        ]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[
            {"role": "SYSTEM", "content": [{"type": "TEXT", "text": "sys"}]},
            {"role": "USER", "content": [{"type": "TEXT", "text": "hi"}]},
            {"role": "ASSISTANT", "content": [{"type": "TEXT", "text": "there"}]}
        ]"#,
    );
}

#[test]
fn generic_messages_convert_text_parts_and_image_url_forms() {
    let adapted = adapt_generic_messages(&messages(
        r#"[{"role": "user", "content": [
            {"type": "text", "text": "look"},
            {"type": "image_url", "image_url": "https://example.com/a.png"},
            {"type": "image_url", "image_url": {"url": "https://example.com/b.png", "detail": "HIGH"}}
        ]}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"role": "USER", "content": [
            {"type": "TEXT", "text": "look"},
            {"type": "IMAGE", "imageUrl": {"url": "https://example.com/a.png"}},
            {"type": "IMAGE", "imageUrl": {"url": "https://example.com/b.png"}}
        ]}]"#,
    );
}

#[test]
fn generic_messages_reject_an_unsupported_content_type() {
    let error = adapt_generic_messages(&messages(
        r#"[{"role": "user", "content": [{"type": "audio", "audio_url": "x"}]}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Content type `audio` is not supported by OCI"
    );
}

#[rstest]
#[case::number(r#"[{"role": "user", "content": [{"type": "text", "text": 5}]}]"#)]
#[case::missing(r#"[{"role": "user", "content": [{"type": "text"}]}]"#)]
fn generic_messages_reject_a_text_part_without_a_string_text(#[case] message: &str) {
    let error = adapt_generic_messages(&messages(message)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Content item of type `text` must have a string `text` field"
    );
}

#[rstest]
#[case::number(r#"[{"role": "user", "content": [{"type": "image_url", "image_url": 5}]}]"#)]
#[case::missing(r#"[{"role": "user", "content": [{"type": "image_url"}]}]"#)]
#[case::object_without_url(
    r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"detail": "LOW"}}]}]"#
)]
fn generic_messages_reject_an_image_url_without_a_url(#[case] content: &str) {
    let error = adapt_generic_messages(&messages(content)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Prop `image_url` must be a string or an object with a `url` property"
    );
}

#[test]
fn generic_messages_skip_messages_a_branch_does_not_take() {
    let adapted = adapt_generic_messages(&messages(
        r#"[
            {"role": "system"},
            {"role": "developer", "content": "dev"},
            {"role": "function", "name": "f", "content": "out"}
        ]"#,
    ))
    .unwrap();
    assert!(adapted.is_empty());
}

#[test]
fn generic_messages_convert_assistant_tool_calls() {
    let adapted = adapt_generic_messages(&messages(
        r#"[{"role": "assistant", "content": "ignored", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"a\":1}"}}
        ]}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"role": "ASSISTANT", "toolCalls": [
            {"id": "call_1", "type": "FUNCTION", "name": "f", "arguments": "{\"a\":1}"}
        ]}]"#,
    );
}

#[test]
fn generic_messages_keep_an_empty_tool_call_list() {
    let adapted =
        adapt_generic_messages(&messages(r#"[{"role": "assistant", "tool_calls": []}]"#)).unwrap();
    assert_json(&adapted, r#"[{"role": "ASSISTANT", "toolCalls": []}]"#);
}

#[test]
fn generic_messages_reject_a_non_function_tool_call() {
    let error = adapt_generic_messages(&messages(
        r#"[{"role": "assistant", "tool_calls": [
            {"id": "call_1", "type": "custom", "function": {"name": "f", "arguments": "{}"}}
        ]}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(error.message, "OCI only supports function tool calls");
}

#[test]
fn generic_messages_convert_tool_results() {
    let adapted = adapt_generic_messages(&messages(
        r#"[{"role": "tool", "tool_call_id": "call_1", "content": "42"}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"role": "TOOL", "content": [{"type": "TEXT", "text": "42"}], "toolCallId": "call_1"}]"#,
    );
}

#[test]
fn generic_messages_reject_a_tool_result_without_an_id() {
    let error =
        adapt_generic_messages(&messages(r#"[{"role": "tool", "content": "42"}]"#)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Tool result message must have a string `tool_call_id`"
    );
}

#[rstest]
#[case::missing(r#"[{"role": "tool", "tool_call_id": "call_1"}]"#)]
#[case::parts(
    r#"[{"role": "tool", "tool_call_id": "call_1", "content": [{"type": "text", "text": "x"}]}]"#
)]
fn generic_messages_reject_a_tool_result_whose_content_is_not_a_string(#[case] message: &str) {
    let error = adapt_generic_messages(&messages(message)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Tool result message `content` must be a string"
    );
}

#[test]
fn generic_tools_clean_the_schema_and_default_the_description() {
    let adapted = adapt_generic_tools(&tools(
        r##"[{"type": "function", "function": {"name": "f", "parameters": {
            "$defs": {"Inner": {"type": "string", "title": "Inner"}},
            "type": "object",
            "properties": {
                "a": {"$ref": "#/$defs/Inner"},
                "b": {"anyOf": [{"type": "null"}, {"type": "integer", "maximum": 3}]},
                "c": {"type": "array"},
                "d": {"type": "any", "title": "D", "default": null}
            },
            "required": ["a", "zz"]
        }}}]"##,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"type": "FUNCTION", "name": "f", "description": "", "parameters": {
            "type": "object",
            "properties": {
                "a": {"type": "string"},
                "b": {"type": "integer", "maximum": 3},
                "c": {"type": "array", "items": {"type": "object"}},
                "d": {"type": "object"}
            },
            "required": ["a"]
        }}]"#,
    );
}

#[test]
fn generic_tools_default_missing_parameters_to_an_empty_object() {
    let adapted = adapt_generic_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "description": "d"}}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"type": "FUNCTION", "name": "f", "description": "d", "parameters": {}}]"#,
    );
}

#[test]
fn generic_tools_reject_a_non_function_tool() {
    let error = adapt_generic_tools(&tools(r#"[{"type": "custom", "custom": {}}]"#)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(error.message, "OCI only supports function tools");
}

#[test]
fn generic_tools_reject_a_tool_without_a_function_object() {
    let error = adapt_generic_tools(&tools(r#"[{"type": "function"}]"#)).unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(error.message, "Tool `function` must be a dictionary");
}

#[test]
fn generic_tools_reject_non_object_parameters() {
    let error = adapt_generic_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "parameters": "x"}}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Tool `function.parameters` must be an object"
    );
}
#[test]
fn oci_chat_request_payload_skips_none_fields() {
    let payload = OciChatRequestPayload {
        api_format: String::from("GENERIC"),
        messages: vec![OciMessage {
            role: OciRole::User,
            content: Some(vec![]),
            tool_calls: None,
            tool_call_id: None,
        }],
        tools: None,
        is_stream: true,
        num_generations: None,
        max_tokens: Some(16),
        max_completion_tokens: None,
        temperature: None,
        top_p: None,
        stop: Some(vec![String::from("end")]),
        seed: None,
        frequency_penalty: None,
        presence_penalty: None,
        reasoning_effort: None,
        response_format: None,
        tool_choice: None,
        logit_bias: None,
        log_probs: None,
    };
    assert_json(
        &payload,
        r#"{"apiFormat": "GENERIC", "messages": [{"role": "USER", "content": []}],
            "isStream": true, "maxTokens": 16, "stop": ["end"]}"#,
    );
}

#[test]
fn oci_vendor_parses_the_wire_values() {
    let vendor: OciVendor = serde_json::from_str(r#""GENERIC""#).unwrap();
    assert_eq!(vendor, OciVendor::Generic);
    let error = serde_json::from_str::<OciVendor>(r#""OTHER""#).unwrap_err();
    assert!(!error.to_string().is_empty());
}
