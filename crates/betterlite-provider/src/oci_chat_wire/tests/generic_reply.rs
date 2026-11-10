use betterlite_openai::chat::response::ChatResponse;
use rstest::rstest;

use crate::oci_chat_wire::generic::reply::{handle_generic_response, handle_generic_stream_chunk};

use super::support::{base_response, body, expected_call_id};

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
fn generic_response_rejects_a_negative_token_count() {
    let mut response = base_response();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0}],
                "usage": {"promptTokens": 1, "completionTokens": -1, "totalTokens": 1}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 200);
    assert!(
        error
            .message
            .starts_with("Response cannot be casted to OCICompletionResponse:")
    );
}

#[test]
fn generic_stream_rejects_an_index_past_u32() {
    let error = handle_generic_stream_chunk(body(r#"{"index": 4294967296}"#)).unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as OCIStreamChunk:")
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

#[test]
fn generic_response_keeps_no_tool_calls_for_an_empty_list() {
    let mut response = base_response();
    handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "2025-01-01T00:00:00Z",
                "choices": [{"index": 0, "message": {"role": "ASSISTANT", "toolCalls": []}}],
                "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        200,
        &mut response,
    )
    .unwrap();
    assert!(response.choices[0].message.tool_calls.is_none());
}

#[test]
fn generic_response_rejects_a_time_created_before_the_unix_epoch() {
    let mut response = base_response();
    let error = handle_generic_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "apiFormat": "GENERIC", "timeCreated": "1969-12-31T23:59:59Z",
                "choices": [{"index": 0}], "usage": {"promptTokens": 1, "totalTokens": 1}
            }}"#,
        ),
        400,
        &mut response,
    )
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "OCI returned a timeCreated before the Unix epoch"
    );
}

#[rstest]
#[case::missing_name(
    r#"{"message": {"toolCalls": [{"id": "x", "arguments": "{}"}]}}"#,
    "",
    "{}"
)]
#[case::missing_arguments(r#"{"message": {"toolCalls": [{"id": "x", "name": "f"}]}}"#, "f", "")]
fn generic_stream_fills_each_missing_tool_call_field(
    #[case] chunk: &str,
    #[case] name: &str,
    #[case] arguments: &str,
) {
    let choice = handle_generic_stream_chunk(body(chunk)).unwrap();
    let call = &choice.delta.tool_calls.as_ref().unwrap()[0];
    let function = call.function.as_ref().unwrap();
    assert_eq!(function.name.as_deref(), Some(name));
    assert_eq!(function.arguments, arguments);
}
