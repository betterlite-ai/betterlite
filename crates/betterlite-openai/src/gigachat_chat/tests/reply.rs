#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::Value;

use super::support::Fixture;
use crate::chat::response::ChatResponse;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::reply::{ReplyInput, transform_response};

fn reply(
    fixture: &Fixture,
    body: &str,
    structured_output: bool,
) -> Result<ChatResponse, GigaChatError> {
    transform_response(
        &fixture.ids,
        &ReplyInput {
            status_code: 200,
            body: body.as_bytes(),
            model: "gigachat/GigaChat",
            created: 1_700_000_000,
            structured_output,
        },
    )
}

#[test]
fn a_plain_reply_keeps_the_reply_fields() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"id": "reply-1", "created": 123, "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.id, "reply-1");
    assert_eq!(response.created, 123);
    assert_eq!(response.model, "gigachat/GigaChat");
    assert_eq!(response.object, "chat.completion");
    let choice = &response.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.usage.prompt_tokens, 3);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(response.usage.prompt_tokens_details, None);
}

#[test]
fn a_missing_id_gets_a_generated_chat_completion_id() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(&fixture, "{}", false).unwrap();
    assert_eq!(response.id, "chatcmpl-0123456789ab");
}

#[test]
fn a_missing_created_uses_the_supplied_clock() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(&fixture, r#"{"choices": []}"#, false).unwrap();
    assert_eq!(response.created, 1_700_000_000);
}

#[test]
fn a_null_created_falls_back_to_the_supplied_clock() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(&fixture, r#"{"created": null}"#, false).unwrap();
    assert_eq!(response.created, 1_700_000_000);
}

#[test]
fn a_missing_finish_reason_becomes_stop() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"content": "hi"}}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
}

#[test]
fn a_finish_reason_outside_the_openai_set_is_mapped_and_kept_as_native() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"content": "hi"}, "finish_reason": "blacklist"}]}"#,
        false,
    )
    .unwrap();
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        choice.provider_specific_fields,
        BTreeMap::from([(
            String::from("native_finish_reason"),
            Value::from("blacklist")
        )])
    );
}

#[test]
fn an_openai_finish_reason_leaves_the_provider_fields_empty() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"content": "hi"}, "finish_reason": "length"}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("length"));
    assert!(response.choices[0].provider_specific_fields.is_empty());
}

#[test]
fn a_missing_role_becomes_assistant() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"content": "hi"}}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.role, "assistant");
}

#[test]
fn a_reply_without_choices_has_no_choices() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(&fixture, r#"{"id": "reply-1"}"#, false).unwrap();
    assert!(response.choices.is_empty());
}

#[test]
fn usage_with_precached_tokens_reports_cached_details() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14, "precached_prompt_tokens": 6}}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 16);
    assert_eq!(response.usage.total_tokens, 20);
    assert_eq!(
        response.usage.prompt_tokens_details,
        Some(BTreeMap::from([(String::from("cached_tokens"), Some(6))]))
    );
}

#[test]
fn a_function_call_reply_becomes_tool_calls() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"role": "assistant", "function_call": {"name": "lookup", "arguments": {"city": "prague"}}}, "finish_reason": "function_call"}]}"#,
        false,
    )
    .unwrap();
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_0123456789ab4def81234567");
    assert_eq!(tool_calls[0].kind, "function");
    assert_eq!(tool_calls[0].function.name, "lookup");
    assert_eq!(tool_calls[0].function.arguments, r#"{"city":"prague"}"#);
    assert_eq!(choice.message.content, None);
}

#[test]
fn a_structured_function_call_reply_becomes_message_content() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"function_call": {"name": "city", "arguments": {"name": "prague"}}}, "finish_reason": "function_call"}]}"#,
        true,
    )
    .unwrap();
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        choice.message.content.as_deref(),
        Some(r#"{"name":"prague"}"#)
    );
    assert_eq!(choice.message.tool_calls, None);
}

#[test]
fn a_structured_reply_without_arguments_writes_an_empty_object() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"function_call": {"name": "city"}}, "finish_reason": "function_call"}]}"#,
        true,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.content.as_deref(), Some("{}"));
}

#[test]
fn a_function_call_on_another_finish_reason_is_ignored() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"function_call": {"name": "lookup"}}, "finish_reason": "stop"}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.choices[0].message.tool_calls, None);
}

#[test]
fn an_empty_function_call_is_ignored() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"function_call": {}}, "finish_reason": "function_call"}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("function_call")
    );
    assert_eq!(response.choices[0].message.tool_calls, None);
}

#[test]
fn tool_calls_in_the_reply_are_kept() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"tool_calls": [{"id": "call_9", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}}]}"#,
        false,
    )
    .unwrap();
    assert_eq!(
        response.choices[0].message.tool_calls.as_ref().unwrap()[0].id,
        "call_9"
    );
}

#[test]
fn an_invalid_json_reply_fails_with_the_upstream_status() {
    let fixture = Fixture::issuing("token", &[]);
    let error = transform_response(
        &fixture.ids,
        &ReplyInput {
            status_code: 502,
            body: b"{not json",
            model: "gigachat/GigaChat",
            created: 0,
            structured_output: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.status_code, 502);
}

#[rstest::rstest]
#[case::array_root("[]")]
#[case::choices_not_a_list(r#"{"choices": "nope"}"#)]
#[case::content_not_a_string(r#"{"choices": [{"message": {"content": 5}}]}"#)]
fn a_reply_of_the_wrong_shape_fails_with_500(#[case] body: &str) {
    let fixture = Fixture::issuing("token", &[]);
    let error = reply(&fixture, body, false).unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.source.is_some());
}

#[test]
fn a_reply_object_with_an_error_key_is_still_read_as_a_reply() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"error": {"message": "no"}, "choices": []}"#,
        false,
    )
    .unwrap();
    assert!(response.choices.is_empty());
    assert_eq!(response.usage.total_tokens, 0);
}

#[test]
fn an_extra_message_field_is_not_forwarded() {
    let fixture = Fixture::issuing("token", &[]);
    let response = reply(
        &fixture,
        r#"{"choices": [{"message": {"content": "hi", "functions_state_id": "state-1"}}]}"#,
        false,
    )
    .unwrap();
    let message = serde_json::to_value(&response.choices[0].message).unwrap();
    assert_eq!(message.get("functions_state_id"), None);
    assert_eq!(message.get("content"), Some(&Value::from("hi")));
}
