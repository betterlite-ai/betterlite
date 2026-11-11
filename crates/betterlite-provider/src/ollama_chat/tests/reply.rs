use rstest::rstest;

use super::super::reply::{
    AssistantMessage, OllamaChatResponse, ReplyInput, ToolCall, transform_response,
};
use super::super::usage::Usage;
use super::support::{FailingCounter, SequenceIds, WordsCounter, messages, options};

const CREATED: i64 = 1_700_000_000;

async fn reply_with(body: &str) -> Result<OllamaChatResponse, super::super::error::OllamaError> {
    reply_with_params(body, "{}", "{}").await
}

async fn reply_with_params(
    body: &str,
    request_data: &str,
    litellm_params: &str,
) -> Result<OllamaChatResponse, super::super::error::OllamaError> {
    let parsed_messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let request_map = options(request_data);
    let params = options(litellm_params);
    let input = ReplyInput {
        model: "llama3",
        body: body.as_bytes(),
        request_data: &request_map,
        litellm_params: &params,
        messages: &parsed_messages,
        created: CREATED,
    };
    transform_response(&input, &WordsCounter, &SequenceIds::new()).await
}

fn usage(prompt_tokens: u64, completion_tokens: u64) -> Usage {
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    }
}

#[tokio::test]
async fn a_text_reply_carries_model_content_finish_reason_and_usage() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "hello"}, "done_reason": "stop", "prompt_eval_count": 3, "eval_count": 4}"#,
    )
    .await
    .unwrap();
    assert_eq!(response.model, "ollama_chat/llama3");
    assert_eq!(response.created, CREATED);
    assert_eq!(response.usage, usage(3, 4));
    assert_eq!(response.choices.len(), 1);
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, "stop");
    assert_eq!(
        choice.message,
        AssistantMessage {
            role: String::from("assistant"),
            content: Some(String::from("hello")),
            reasoning_content: None,
            tool_calls: None,
        }
    );
}

#[tokio::test]
async fn missing_counts_fall_back_to_the_injected_token_counter() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "hello world"}, "done": true}"#,
    )
    .await
    .unwrap();
    assert_eq!(response.usage, usage(1, 2));
}

#[tokio::test]
async fn a_failing_token_counter_counts_zero() {
    let parsed_messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let request_data = options("{}");
    let litellm_params = options("{}");
    let input = ReplyInput {
        model: "llama3",
        body: br#"{"model": "llama3", "message": {"role": "assistant", "content": "hello"}}"#,
        request_data: &request_data,
        litellm_params: &litellm_params,
        messages: &parsed_messages,
        created: CREATED,
    };
    let response = transform_response(&input, &FailingCounter, &SequenceIds::new())
        .await
        .unwrap();
    assert_eq!(response.usage, usage(0, 0));
}

#[tokio::test]
async fn a_thinking_message_becomes_reasoning_content() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "hello", "thinking": "why"}}"#,
    )
    .await
    .unwrap();
    let message = &response.choices.first().unwrap().message;
    assert_eq!(message.reasoning_content.as_deref(), Some("why"));
    assert_eq!(message.content.as_deref(), Some("hello"));
}

#[tokio::test]
async fn an_empty_thinking_is_kept_as_empty_reasoning() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "hello", "thinking": ""}}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .choices
            .first()
            .unwrap()
            .message
            .reasoning_content
            .as_deref(),
        Some("")
    );
}

#[tokio::test]
async fn a_null_thinking_stops_the_content_from_being_parsed() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "<think>inner</think>answer", "thinking": null}}"#,
    )
    .await
    .unwrap();
    let message = &response.choices.first().unwrap().message;
    assert_eq!(message.reasoning_content, None);
    assert_eq!(
        message.content.as_deref(),
        Some("<think>inner</think>answer")
    );
}

#[tokio::test]
async fn think_tags_in_content_become_reasoning() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "<think>inner</think>answer"}}"#,
    )
    .await
    .unwrap();
    let message = &response.choices.first().unwrap().message;
    assert_eq!(message.reasoning_content.as_deref(), Some("inner"));
    assert_eq!(message.content.as_deref(), Some("answer"));
}

#[rstest]
#[case::length(r#""length""#, "length")]
#[case::unknown(r#""load""#, "stop")]
#[case::empty(r#""""#, "stop")]
#[case::missing_null("null", "stop")]
#[tokio::test]
async fn the_finish_reason_is_mapped(#[case] reason: &str, #[case] expected: &str) {
    let body = format!(
        r#"{{"model": "llama3", "message": {{"role": "assistant", "content": "x"}}, "done_reason": {reason}}}"#
    );
    let response = reply_with(&body).await.unwrap();
    assert_eq!(response.choices.first().unwrap().finish_reason, expected);
}

#[tokio::test]
async fn a_null_content_stays_null() {
    let response =
        reply_with(r#"{"model": "llama3", "message": {"role": "assistant", "content": null}}"#)
            .await
            .unwrap();
    assert_eq!(response.choices.first().unwrap().message.content, None);
}

#[tokio::test]
async fn a_reply_without_a_content_key_stays_null() {
    let response = reply_with(r#"{"model": "llama3", "message": {"role": "assistant"}}"#)
        .await
        .unwrap();
    assert_eq!(response.choices.first().unwrap().message.content, None);
}

#[tokio::test]
async fn a_missing_role_defaults_to_assistant() {
    let response = reply_with(r#"{"model": "llama3", "message": {"content": "x"}}"#)
        .await
        .unwrap();
    assert_eq!(response.choices.first().unwrap().message.role, "assistant");
}

#[tokio::test]
async fn tool_calls_are_converted_with_json_arguments() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "tool_calls": [{"function": {"name": "f", "arguments": {"a": 1}}}, {"function": {"name": "g", "arguments": "{\"z\": 1}"}}, {"function": {"name": "h"}}]}}"#,
    )
    .await
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.finish_reason, "tool_calls");
    let calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(
        calls.first().unwrap(),
        &ToolCall {
            id: String::from("id-0"),
            kind: String::from("function"),
            function: super::super::reply::Function {
                name: Some(String::from("f")),
                arguments: String::from(r#"{"a":1}"#),
            },
        }
    );
    assert_eq!(
        calls.get(1).map(|call| call.function.arguments.as_str()),
        Some(r#"{"z": 1}"#)
    );
    assert_eq!(
        calls.get(2).map(|call| call.function.arguments.as_str()),
        Some("")
    );
}

#[tokio::test]
async fn tool_call_parameters_fill_missing_arguments() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "tool_calls": [{"function": {"name": "f", "parameters": {"p": 1}}}]}}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .choices
            .first()
            .unwrap()
            .message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .map(|call| call.function.arguments.as_str()),
        Some(r#"{"p":1}"#)
    );
}

#[tokio::test]
async fn a_null_tool_call_type_is_a_function() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "tool_calls": [{"type": null, "function": {"name": "f", "arguments": "{}"}}]}}"#,
    )
    .await
    .unwrap();
    let call = response
        .choices
        .first()
        .unwrap()
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.kind, "function");
}

#[tokio::test]
async fn a_given_tool_call_id_and_type_are_kept() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "tool_calls": [{"id": "given", "type": "other", "function": {"name": "f", "arguments": "{}"}}]}}"#,
    )
    .await
    .unwrap();
    let call = response
        .choices
        .first()
        .unwrap()
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.id, "given");
    assert_eq!(call.kind, "other");
}

#[tokio::test]
async fn an_empty_tool_call_list_keeps_the_mapped_finish_reason() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "tool_calls": []}, "done_reason": "length"}"#,
    )
    .await
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.finish_reason, "length");
    assert_eq!(choice.message.tool_calls, None);
}

#[tokio::test]
async fn json_mode_turns_the_content_into_a_tool_call() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{\"name\":\"other\",\"arguments\":{\"a\":1}}"}}"#,
        r#"{"format": "json"}"#,
        r#"{"function_name": "f"}"#,
    )
    .await
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.finish_reason, "tool_calls");
    assert_eq!(choice.message.content, None);
    let call = choice
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.id, "call_id-0");
    assert_eq!(call.function.name.as_deref(), Some("other"));
    assert_eq!(call.function.arguments, r#"{"a":1}"#);
}

#[tokio::test]
async fn json_mode_falls_back_to_the_function_name_and_the_whole_object() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{\"a\":1}"}}"#,
        r#"{"format": "json"}"#,
        r#"{"function_name": "f"}"#,
    )
    .await
    .unwrap();
    let call = response
        .choices
        .first()
        .unwrap()
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.function.name.as_deref(), Some("f"));
    assert_eq!(call.function.arguments, r#"{"a":1}"#);
}

#[tokio::test]
async fn json_mode_keeps_a_null_name() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{\"name\":null,\"arguments\":{}}"}}"#,
        r#"{"format": "json"}"#,
        r#"{"function_name": "f"}"#,
    )
    .await
    .unwrap();
    let call = response
        .choices
        .first()
        .unwrap()
        .message
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .unwrap();
    assert_eq!(call.function.name, None);
    assert_eq!(call.function.arguments, "{}");
}

#[tokio::test]
async fn json_mode_carries_reasoning() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{\"name\":\"n\"}", "thinking": "t"}}"#,
        r#"{"format": "json"}"#,
        r#"{"function_name": "f"}"#,
    )
    .await
    .unwrap();
    let message = &response.choices.first().unwrap().message;
    assert_eq!(message.reasoning_content.as_deref(), Some("t"));
    assert_eq!(
        message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .map(|call| call.function.arguments.as_str()),
        Some(r#"{"name":"n"}"#)
    );
}

#[tokio::test]
async fn a_json_format_without_a_function_name_is_not_json_mode() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{\"name\":\"n\"}"}}"#,
        r#"{"format": "json"}"#,
        "{}",
    )
    .await
    .unwrap();
    let choice = response.choices.first().unwrap();
    assert_eq!(choice.finish_reason, "stop");
    assert_eq!(choice.message.content.as_deref(), Some("{\"name\":\"n\"}"));
}

#[tokio::test]
async fn a_non_json_format_is_not_json_mode() {
    let response = reply_with_params(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "{}"}}"#,
        r#"{"format": {"type": "object"}}"#,
        r#"{"function_name": "f"}"#,
    )
    .await
    .unwrap();
    assert_eq!(response.choices.first().unwrap().finish_reason, "stop");
}

#[rstest]
#[case::invalid_json(
    r#"{"model": "llama3", "message": {"role": "assistant", "content": "not json"}}"#
)]
#[case::not_an_object(
    r#"{"model": "llama3", "message": {"role": "assistant", "content": "[1, 2]"}}"#
)]
#[case::null_content(r#"{"model": "llama3", "message": {"role": "assistant", "content": null}}"#)]
#[tokio::test]
async fn json_mode_rejects_content_that_is_not_a_json_object(#[case] body: &str) {
    let error = reply_with_params(body, r#"{"format": "json"}"#, r#"{"function_name": "f"}"#)
        .await
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_reply_of_the_wrong_shape_has_no_status() {
    let error = reply_with(r#"{"model": "llama3"}"#).await.unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_reply_that_is_not_json_has_no_status() {
    let error = reply_with("not json").await.unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_message_of_the_wrong_type_has_no_status() {
    let error = reply_with(r#"{"model": "llama3", "message": 5}"#)
        .await
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_content_of_the_wrong_type_has_no_status() {
    let error =
        reply_with(r#"{"model": "llama3", "message": {"role": "assistant", "content": 5}}"#)
            .await
            .unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn an_unknown_tool_call_field_is_ignored() {
    let response = reply_with(
        r#"{"model": "llama3", "message": {"role": "assistant", "content": "x", "extra": 1, "tool_calls": [{"index": 3, "function": {"name": "f", "arguments": {}}}]}}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .choices
            .first()
            .unwrap()
            .message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .map(|call| call.function.name.as_deref()),
        Some(Some("f"))
    );
}

#[tokio::test]
async fn the_response_is_debuggable() {
    let response =
        reply_with(r#"{"model": "llama3", "message": {"role": "assistant", "content": "x"}}"#)
            .await
            .unwrap();
    assert!(format!("{response:?}").contains("ollama_chat/llama3"));
    assert_eq!(
        response.choices.first().unwrap().message.content,
        Some(String::from("x"))
    );
}
