#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{ChatMessage, MessageContent, Tool};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::oci_chat_wire::cohere::reply::handle_cohere_stream_chunk;
use crate::oci_chat_wire::cohere::reply::{CohereEmitted, handle_cohere_response};
use crate::oci_chat_wire::cohere::request::{
    adapt_cohere_messages, adapt_cohere_tools, extract_text_content,
};
use crate::oci_chat_wire::generic::reply::{handle_generic_response, handle_generic_stream_chunk};
use crate::oci_chat_wire::generic::request::{adapt_generic_messages, adapt_generic_tools};
use crate::oci_chat_wire::ids::{normalize_oci_finish_reason, synthesize_oci_tool_call_id};
use crate::oci_chat_wire::types::oci::{OciChatRequestPayload, OciMessage, OciRole, OciVendor};

fn body(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

fn tools(text: &str) -> Vec<Tool> {
    serde_json::from_str(text).unwrap()
}

fn assert_json<T>(value: &T, expected_text: &str)
where
    T: Serialize,
{
    let actual = serde_json::to_value(value).unwrap();
    let expected = serde_json::to_value(body(expected_text)).unwrap();
    assert_eq!(actual, expected);
}

fn base_response() -> ChatResponse {
    ChatResponse {
        id: String::from("chatcmpl-oci"),
        object: String::from("chat.completion"),
        created: 0,
        model: String::from("before"),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: None,
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn expected_call_id(position: usize, name: &str, sorted: &str) -> String {
    let mut digest = hex::encode(Sha256::digest(format!("{position}|{name}|{sorted}")));
    digest.truncate(24);
    format!("call_{digest}")
}

#[test]
fn vendor_api_format_uses_generic_for_any_non_cohere_vendor() {
    assert_eq!(OciVendor::Cohere.api_format(), "COHERE");
    assert_eq!(OciVendor::Generic.api_format(), "GENERIC");
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

#[test]
fn generic_messages_reject_a_text_part_without_a_string_text() {
    let error = adapt_generic_messages(&messages(
        r#"[{"role": "user", "content": [{"type": "text", "text": 5}]}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Content item of type `text` must have a string `text` field"
    );
}

#[rstest]
#[case::number(r#"[{"role": "user", "content": [{"type": "image_url", "image_url": 5}]}]"#)]
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
fn generic_response_fills_text_tools_finish_reason_and_usage() {
    let mut response = base_response();
    handle_generic_response(
        &body(
            r#"{"modelId": "meta.llama", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC",
                "timeCreated": "2025-01-01T00:00:00.000Z",
                "choices": [{"index": 0, "finishReason": "MAX_TOKENS", "message": {
                    "role": "ASSISTANT",
                    "content": [
                        {"type": "TEXT", "text": "he"},
                        {"type": "IMAGE", "imageUrl": {"url": "https://example.com/x.png"}},
                        {"type": "TEXT", "text": "llo"}
                    ],
                    "toolCalls": [
                        {"id": "call_given", "type": "FUNCTION", "name": "f", "arguments": "{}"},
                        {"id": "", "type": "FUNCTION", "name": "g", "arguments": "{\"b\":2}"}
                    ]
                }}],
                "usage": {
                    "promptTokens": 3, "completionTokens": 5, "totalTokens": 8,
                    "completionTokensDetails": {"reasoningTokens": 2}
                }
            }}"#,
        ),
        400,
        &mut response,
    )
    .unwrap();
    assert_filled_response(&response);
}

fn assert_filled_response(response: &ChatResponse) {
    assert_eq!(response.created, 1_735_689_600);
    assert_eq!(response.model, "meta.llama");
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].id, "call_given");
    assert_eq!(tool_calls[0].kind, "function");
    assert_eq!(tool_calls[0].function.name, "f");
    assert_eq!(tool_calls[1].id, expected_call_id(1, "g", "{\"b\":2}"));
    assert_eq!(tool_calls[1].function.arguments, "{\"b\":2}");
    assert_eq!(response.usage.prompt_tokens, 3);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 8);
    let details = response.usage.completion_tokens_details.as_ref().unwrap();
    assert_eq!(details.get("reasoning_tokens"), Some(&Some(2)));
    assert_eq!(details.get("text_tokens"), Some(&Some(3)));
}

#[test]
fn generic_response_keeps_content_when_the_message_has_no_text_part() {
    let mut response = base_response();
    response.choices[0].message.content = Some(String::from("keep"));
    handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0, "message": {"role": "ASSISTANT", "content": [
                    {"type": "IMAGE", "imageUrl": {"url": "https://example.com/x.png"}}
                ]}}],
                "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        500,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.content.as_deref(), Some("keep"));
    assert_eq!(response.choices[0].finish_reason, None);
}

#[test]
fn generic_response_without_message_keeps_the_premade_message() {
    let mut response = base_response();
    response.choices[0].message.content = Some(String::from("keep"));
    handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0, "finishReason": "COMPLETE"}],
                "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.content.as_deref(), Some("keep"));
    assert_eq!(response.choices[0].message.tool_calls, None);
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
}

#[test]
fn generic_response_defaults_completion_tokens_to_zero() {
    let mut response = base_response();
    handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0, "message": {"role": "ASSISTANT", "content": []}}],
                "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.usage.completion_tokens, 0);
    assert_eq!(response.usage.completion_tokens_details, None);
    assert_eq!(response.choices[0].message.content, None);
}

#[test]
fn generic_response_rejects_a_body_without_choices() {
    let mut response = base_response();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [], "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        529,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 529);
    assert_eq!(error.message, "OCI response contained no choices");
}

#[test]
fn generic_response_reports_a_body_that_does_not_fit_its_type() {
    let mut response = base_response();
    let error = handle_generic_response(&body("{}"), 502, &mut response).unwrap_err();
    assert_eq!(error.status, 502);
    assert!(
        error
            .message
            .starts_with("Response cannot be casted to OCICompletionResponse:")
    );
}

#[test]
fn generic_response_rejects_an_unknown_content_type() {
    let mut response = base_response();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0, "message": {"role": "ASSISTANT", "content": [
                    {"type": "AUDIO", "text": "x"}
                ]}}],
                "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        418,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 418);
    assert!(
        error
            .message
            .starts_with("Response cannot be casted to OCICompletionResponse:")
    );
}

#[test]
fn generic_response_rejects_an_unreadable_time_created() {
    let mut response = base_response();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01",
                "choices": [{"index": 0}], "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        503,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(
        error.message,
        "OCI returned an unreadable timeCreated value"
    );
}

#[test]
fn generic_response_omits_details_for_zero_reasoning_tokens() {
    let mut response = base_response();
    handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0}],
                "usage": {"promptTokens": 1, "completionTokens": 0, "totalTokens": 1,
                    "completionTokensDetails": {"reasoningTokens": 0}}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.usage.completion_tokens_details, None);
}

#[test]
fn generic_stream_concatenates_text_and_defaults_the_index() {
    let choice = handle_generic_stream_chunk(body(
        r#"{"finishReason": "COMPLETE", "message": {"role": "ASSISTANT", "content": [
            {"type": "TEXT", "text": "a"}, {"type": "TEXT", "text": "b"}
        ]}}"#,
    ))
    .unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.delta.content.as_deref(), Some("ab"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert!(choice.delta.tool_calls.is_none());
}

#[test]
fn generic_stream_without_text_parts_gives_no_content() {
    let choice = handle_generic_stream_chunk(body(
        r#"{"index": 2, "message": {"role": "ASSISTANT", "content": []}}"#,
    ))
    .unwrap();
    assert_eq!(choice.index, 2);
    assert_eq!(choice.delta.content, None);
}

#[test]
fn generic_stream_defaults_missing_tool_call_fields_and_synthesizes_ids() {
    let choice = handle_generic_stream_chunk(body(
        r#"{"message": {"toolCalls": [
            {"id": "call_given", "name": "f", "arguments": "{}"},
            {"arguments": "{\"x\":1}"}
        ]}}"#,
    ))
    .unwrap();
    let tool_calls = choice.delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].index, 0);
    assert_eq!(tool_calls[0].id.as_deref(), Some("call_given"));
    assert_eq!(tool_calls[0].kind.as_deref(), Some("function"));
    assert_eq!(tool_calls[1].index, 1);
    assert_eq!(
        tool_calls[1].id.as_deref(),
        Some(expected_call_id(1, "", "{\"x\":1}").as_str())
    );
    assert_eq!(
        tool_calls[1].function.as_ref().unwrap().name.as_deref(),
        Some("")
    );
}

#[test]
fn generic_stream_synthesizes_an_empty_tool_call_id() {
    let choice = handle_generic_stream_chunk(body(
        r#"{"message": {"toolCalls": [{"id": "", "name": "f", "arguments": "{}"}]}}"#,
    ))
    .unwrap();
    let tool_calls = choice.delta.tool_calls.as_ref().unwrap();
    assert_eq!(
        tool_calls[0].id.as_deref(),
        Some(expected_call_id(0, "f", "{}").as_str())
    );
}

#[test]
fn generic_stream_rejects_image_content() {
    let error = handle_generic_stream_chunk(body(
        r#"{"message": {"content": [{"type": "IMAGE", "imageUrl": {"url": "https://x"}}]}}"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert_eq!(
        error.message,
        "OCI returned image content in a streaming response, which is not supported"
    );
}

#[test]
fn generic_stream_rejects_unknown_content_types() {
    let error = handle_generic_stream_chunk(body(
        r#"{"message": {"content": [{"type": "AUDIO", "text": "x"}]}}"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as OCIStreamChunk:")
    );
}

#[test]
fn generic_stream_normalizes_finish_reasons() {
    let choice = handle_generic_stream_chunk(body(r#"{"finishReason": "TOOL_CALLS"}"#)).unwrap();
    assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn generic_stream_reports_a_chunk_that_does_not_fit_its_type() {
    let error = handle_generic_stream_chunk(body(r#"{"index": "zero"}"#)).unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as OCIStreamChunk:")
    );
}

#[test]
fn generic_stream_leaves_chunks_without_a_message_alone() {
    let choice = handle_generic_stream_chunk(body(r#"{"pad": "x"}"#)).unwrap();
    assert_eq!(choice.delta.content, None);
    assert!(choice.delta.tool_calls.is_none());
    assert_eq!(choice.finish_reason, None);
}

#[rstest]
#[case::none(None, None)]
#[case::complete(Some("COMPLETE"), Some("stop"))]
#[case::max_tokens(Some("MAX_TOKENS"), Some("length"))]
#[case::tool_call(Some("TOOL_CALL"), Some("tool_calls"))]
#[case::tool_calls(Some("TOOL_CALLS"), Some("tool_calls"))]
#[case::cancelled(Some("CANCELLED"), Some("stop"))]
#[case::empty(Some(""), Some("stop"))]
fn finish_reasons_map_to_openai_values(#[case] raw: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(normalize_oci_finish_reason(raw).as_deref(), expected);
}

#[test]
fn synthesized_tool_call_ids_hash_position_name_and_arguments() {
    let id = synthesize_oci_tool_call_id(3, "lookup", "{\"a\":1}");
    assert_eq!(id, expected_call_id(3, "lookup", "{\"a\":1}"));
    assert_eq!(id.len(), 29);
    assert_ne!(id, synthesize_oci_tool_call_id(4, "lookup", "{\"a\":1}"));
    assert_ne!(id, synthesize_oci_tool_call_id(3, "other", "{\"a\":1}"));
}

#[test]
fn cohere_text_content_joins_text_parts_only() {
    let content: MessageContent = serde_json::from_str(
        r#"[{"type": "text", "text": "a"}, {"type": "image_url", "text": "ignored", "image_url": "x"}, {"type": "text", "text": "b"}]"#,
    )
    .unwrap();
    assert_eq!(extract_text_content(Some(&content)), "ab");
    let text: MessageContent = serde_json::from_str(r#""plain""#).unwrap();
    assert_eq!(extract_text_content(Some(&text)), "plain");
    assert_eq!(extract_text_content(None), "");
}

#[test]
fn cohere_messages_build_history_without_the_last_user_message() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "answer"},
            {"role": "user", "content": "last"}
        ]"#,
    ));
    assert_json(
        &history,
        r#"[
            {"role": "USER", "message": "first"},
            {"role": "CHATBOT", "message": "answer"}
        ]"#,
    );
}

#[test]
fn cohere_messages_keep_every_message_when_no_user_message_exists() {
    let history =
        adapt_cohere_messages(&messages(r#"[{"role": "assistant", "content": "answer"}]"#));
    assert_json(&history, r#"[{"role": "CHATBOT", "message": "answer"}]"#);
}

#[test]
fn cohere_messages_parse_tool_call_arguments() {
    let history = adapt_cohere_messages(&messages(
        r#"[{"role": "assistant", "content": "", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"b\": 2, \"a\": 1}"}},
            {"id": "call_2", "type": "function", "function": {"name": "g", "arguments": "not json"}},
            {"id": "call_3", "type": "function", "function": {"name": "h", "arguments": "[]"}}
        ]}]"#,
    ));
    assert_json(
        &history,
        r#"[{"role": "CHATBOT", "message": "", "toolCalls": [
            {"name": "f", "parameters": {"b": 2, "a": 1}},
            {"name": "g", "parameters": {}},
            {"name": "h", "parameters": {}}
        ]}]"#,
    );
}

#[test]
fn cohere_messages_merge_consecutive_tool_results_and_look_up_calls() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"a\": 1}"}}
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "one"},
            {"role": "tool", "tool_call_id": "unknown", "content": "two"}
        ]"#,
    ));
    assert_json(
        &history,
        r#"[
            {"role": "CHATBOT", "message": "", "toolCalls": [{"name": "f", "parameters": {"a": 1}}]},
            {"role": "TOOL", "toolResults": [
                {"call": {"name": "f", "parameters": {"a": 1}}, "outputs": [{"output": "one"}]},
                {"call": {"name": "", "parameters": {}}, "outputs": [{"output": "two"}]}
            ]}
        ]"#,
    );
}

#[test]
fn cohere_messages_keep_separated_tool_results_apart() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "tool", "tool_call_id": "a", "content": "one"},
            {"role": "user", "content": "middle"},
            {"role": "assistant", "content": "x"},
            {"role": "tool", "tool_call_id": "b", "content": "two"}
        ]"#,
    ));
    let json = serde_json::to_value(&history).unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["role"], "TOOL");
    assert_eq!(items[1]["role"], "CHATBOT");
    assert_eq!(items[2]["role"], "TOOL");
}

#[test]
fn cohere_messages_look_up_tool_results_across_all_assistant_messages() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "assistant", "content": "a", "tool_calls": [
                {"id": "early", "type": "function", "function": {"name": "one", "arguments": "{}"}}
            ]},
            {"role": "assistant", "content": "b", "tool_calls": [
                {"id": "late", "type": "function", "function": {"name": "two", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "early", "content": "result"},
            {"role": "user", "content": "next"}
        ]"#,
    ));
    let json = serde_json::to_value(&history).unwrap();
    assert_eq!(json[2]["toolResults"][0]["call"]["name"], "one");
}

#[test]
fn cohere_messages_skip_system_developer_and_function_roles() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "developer", "content": "dev"},
            {"role": "function", "name": "f", "content": "out"}
        ]"#,
    ));
    assert!(history.is_empty());
}

#[test]
fn cohere_tools_map_parameter_definitions() {
    let adapted = adapt_cohere_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "description": "desc", "parameters": {
            "type": "object",
            "properties": {
                "s": {"type": "string", "description": "S", "enum": ["a", 1]},
                "i": {"type": "integer", "minimum": 1, "maximum": 9},
                "u": {"type": "weird"},
                "n": {},
                "x": {"type": "string", "description": "X"}
            },
            "required": ["s", "n", "missing", 7]
        }}}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"name": "f", "description": "desc", "parameterDefinitions": {
            "s": {"description": "S. Allowed values: ['a', 1]", "type": "str", "isRequired": true},
            "i": {"description": "Range: min=1, max=9", "type": "int", "isRequired": false},
            "u": {"description": "", "type": "weird", "isRequired": false},
            "n": {"description": "", "type": "string", "isRequired": false},
            "x": {"description": "X", "type": "str", "isRequired": false}
        }}]"#,
    );
}

#[test]
fn cohere_tools_default_name_description_and_properties() {
    let adapted = adapt_cohere_tools(&tools(
        r#"[{"type": "custom", "custom": {}}, {"type": "function", "function": {"name": "g"}}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[
            {"name": "", "description": "", "parameterDefinitions": {}},
            {"name": "g", "description": "", "parameterDefinitions": {}}
        ]"#,
    );
}

#[test]
fn cohere_tools_reject_non_object_parameters() {
    let error = adapt_cohere_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "parameters": []}}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Tool `function.parameters` must be an object"
    );
}

#[test]
fn cohere_response_fills_text_model_created_and_usage() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "cohere.command-r", "modelVersion": "1.0", "chatResponse": {
                "text": "hello", "apiFormat": "COHERE", "finishReason": "COMPLETE",
                "usage": {"promptTokens": 2, "completionTokens": 3, "totalTokens": 5}
            }}"#,
        ),
        "cohere.command-r",
        1_700_000_000,
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.model, "cohere.command-r");
    assert_eq!(response.created, 1_700_000_000);
    let choice = &response.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    assert_eq!(choice.message.tool_calls, None);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.completion_tokens, 3);
    assert_eq!(response.usage.total_tokens, 5);
}

#[test]
fn cohere_response_turns_empty_text_into_no_content() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {"text": "", "apiFormat": "COHERE"}}"#,
        ),
        "m",
        0,
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.content, None);
    assert_eq!(response.usage, ChatUsage::default());
}

#[test]
fn cohere_response_keeps_parameters_order_and_hashes_sorted_keys() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "text": "", "apiFormat": "COHERE", "finishReason": "TOOL_CALL",
                "toolCalls": [{"name": "f", "parameters": {"b": 2, "a": 1}}]
            }}"#,
        ),
        "m",
        0,
        200,
        &mut response,
    )
    .unwrap();
    let tool_calls = response.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].function.arguments, "{\"b\":2,\"a\":1}");
    assert_eq!(
        tool_calls[0].id,
        expected_call_id(0, "f", "{\"a\":1,\"b\":2}")
    );
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[test]
fn cohere_response_reports_a_body_that_does_not_fit_its_type() {
    let mut response = base_response();
    let error = handle_cohere_response(&body("{}"), "m", 0, 401, &mut response).unwrap_err();
    assert_eq!(error.status, 401);
    assert!(
        error
            .message
            .starts_with("Response cannot be casted to CohereChatResult:")
    );
}

#[test]
fn cohere_stream_emits_text_and_defaults_the_index() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "text": "hi", "finishReason": "COMPLETE"}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn cohere_stream_drops_restated_text_after_prior_text() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "again", "chatHistory": [{"role": "USER", "message": "x"}]}"#,
        ),
        CohereEmitted {
            tool_calls: false,
            text: true,
        },
    )
    .unwrap();
    assert_eq!(choice.delta.content, None);
}

#[test]
fn cohere_stream_drops_text_restated_by_a_tool_call_chunk() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "again", "toolCalls": [{"name": "f", "parameters": {}}]}"#,
        ),
        CohereEmitted {
            tool_calls: false,
            text: true,
        },
    )
    .unwrap();
    assert_eq!(choice.delta.content, None);
}

#[test]
fn cohere_stream_keeps_restated_text_without_prior_text() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "only", "chatHistory": [{"role": "USER", "message": "x"}]}"#,
        ),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.delta.content.as_deref(), Some("only"));
}

#[test]
fn cohere_stream_drops_restated_tool_calls_after_prior_calls() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "chatHistory": [{"role": "USER", "message": "x"}],
                "toolCalls": [{"name": "f", "parameters": {"a": 1}}]}"#,
        ),
        CohereEmitted {
            tool_calls: true,
            text: false,
        },
    )
    .unwrap();
    assert!(choice.delta.tool_calls.is_none());
}

#[test]
fn cohere_stream_keeps_tool_calls_that_are_not_restated() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "toolCalls": [{"name": "f", "parameters": {"a": 1}}]}"#),
        CohereEmitted {
            tool_calls: true,
            text: false,
        },
    )
    .unwrap();
    let tool_calls = choice.delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].index, 0);
    assert_eq!(tool_calls[0].kind.as_deref(), Some("function"));
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().arguments,
        "{\"a\":1}"
    );
    assert_eq!(
        tool_calls[0].id.as_deref(),
        Some(expected_call_id(0, "f", "{\"a\":1}").as_str())
    );
}

#[test]
fn cohere_stream_keeps_tool_calls_without_prior_calls() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "chatHistory": [{"role": "USER", "message": "x"}],
                "toolCalls": [{"name": "f", "parameters": {}}]}"#,
        ),
        CohereEmitted::default(),
    )
    .unwrap();
    assert!(choice.delta.tool_calls.is_some());
}

#[test]
fn cohere_stream_normalizes_an_unknown_finish_reason_to_stop() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "finishReason": "USER_CANCEL"}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn cohere_stream_keeps_an_explicit_index() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "index": 4}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.index, 4);
}

#[test]
fn cohere_stream_reports_a_chunk_that_does_not_fit_its_type() {
    let error = handle_cohere_stream_chunk(&body(r#"{"text": "x"}"#), CohereEmitted::default())
        .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as CohereStreamChunk:")
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

#[test]
fn generic_response_reports_a_response_without_a_choice_to_fill() {
    let mut response = base_response();
    response.choices.clear();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0}], "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 200);
    assert_eq!(error.message, "the response to fill has no choices");
}
