use indexmap::IndexMap;
use rstest::rstest;

use crate::oci_chat::response::{MAX_CHAT_BODY_BYTES, decode_chat_body, transform_response};
use crate::oci_chat::tests::support::{FixedClock, FixedIds, body, pick, to_json};

const LLAMA: &str = "meta.llama-3.3-70b-instruct";

const GENERIC_BODY: &str = r#"{
    "modelId": "meta.llama",
    "modelVersion": "1",
    "chatResponse": {
        "apiFormat": "GENERIC",
        "timeCreated": "2025-01-01T00:00:00.000Z",
        "choices": [{
            "index": 0,
            "finishReason": "TOOL_CALL",
            "message": {
                "role": "ASSISTANT",
                "content": [{"type": "TEXT", "text": "hi "}, {"type": "TEXT", "text": "there"}],
                "toolCalls": [{"id": "call_1", "type": "FUNCTION", "name": "lookup", "arguments": "{}"}]
            }
        }],
        "usage": {"promptTokens": 3, "completionTokens": 5, "totalTokens": 8}
    }
}"#;

const COHERE_BODY: &str = r#"{
    "modelId": "cohere.command-r",
    "modelVersion": "1.0",
    "chatResponse": {
        "text": "hello",
        "apiFormat": "COHERE",
        "finishReason": "COMPLETE",
        "usage": {"promptTokens": 2, "completionTokens": 3, "totalTokens": 5}
    }
}"#;

#[test]
fn response_decodes_a_generic_body() {
    let reply = transform_response(
        LLAMA,
        200,
        GENERIC_BODY.as_bytes(),
        &IndexMap::new(),
        &FixedClock(0),
        &FixedIds,
    )
    .unwrap();
    assert_eq!(reply.response.id, "chatcmpl-fixed-id");
    assert_eq!(reply.response.model, "meta.llama");
    assert_eq!(reply.response.created, 1_735_689_600);
    let choice = &reply.response.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some("hi there"));
    assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
    let tool_calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].id, "call_1");
    assert_eq!(reply.response.usage.total_tokens, 8);
}

#[test]
fn response_decodes_a_cohere_body() {
    let reply = transform_response(
        "cohere.command-r",
        200,
        COHERE_BODY.as_bytes(),
        &IndexMap::new(),
        &FixedClock(1_700_000_000),
        &FixedIds,
    )
    .unwrap();
    assert_eq!(reply.response.id, "chatcmpl-fixed-id");
    assert_eq!(reply.response.model, "cohere.command-r");
    assert_eq!(reply.response.created, 1_700_000_000);
    let choice = &reply.response.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn response_ignores_a_null_error_key() {
    let with_null_error = GENERIC_BODY.replacen('{', r#"{"error": null,"#, 1);
    let reply = transform_response(
        LLAMA,
        200,
        with_null_error.as_bytes(),
        &IndexMap::new(),
        &FixedClock(0),
        &FixedIds,
    )
    .unwrap();
    assert_eq!(reply.response.model, "meta.llama");
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi there")
    );
}

#[rstest]
#[case::text(502, r#"{"error": "boom"}"#, "boom")]
#[case::object(500, r#"{"error": {"message": "x"}}"#, r#"{"message":"x"}"#)]
#[case::number(400, r#"{"error": 7}"#, "7")]
fn response_maps_the_error_key(
    #[case] status: u16,
    #[case] given: &str,
    #[case] message: &str,
) {
    let refusal = transform_response(
        LLAMA,
        status,
        given.as_bytes(),
        &IndexMap::new(),
        &FixedClock(0),
        &FixedIds,
    )
    .unwrap_err();
    assert_eq!(refusal.status, status);
    assert_eq!(refusal.message, message);
}

#[test]
fn response_rejects_a_body_that_is_not_an_object() {
    let refusal = transform_response(
        LLAMA,
        201,
        b"[]",
        &IndexMap::new(),
        &FixedClock(0),
        &FixedIds,
    )
    .unwrap_err();
    assert_eq!(refusal.status, 201);
    assert_eq!(refusal.message, "Invalid response format from OCI");
}

#[test]
fn response_rejects_bad_json() {
    let refusal = decode_chat_body(200, b"{oops").unwrap_err();
    assert_eq!(refusal.status, 200);
    assert!(
        refusal
            .message
            .starts_with("Invalid response format from OCI:")
    );
}

#[test]
fn response_body_limit_is_inclusive() {
    let mut at_limit = vec![b' '; MAX_CHAT_BODY_BYTES];
    at_limit[..2].copy_from_slice(b"{}");
    assert_eq!(decode_chat_body(200, &at_limit).unwrap(), body("{}"));
    let oversized = vec![b' '; MAX_CHAT_BODY_BYTES.saturating_add(1)];
    let refusal = decode_chat_body(200, &oversized).unwrap_err();
    assert_eq!(refusal.status, 200);
    assert_eq!(
        refusal.message,
        "the OCI chat response body is larger than 16777216 bytes"
    );
}

#[test]
fn response_keeps_the_response_headers() {
    let mut headers = IndexMap::new();
    headers.insert(String::from("x-request-id"), String::from("abc"));
    let reply = transform_response(
        LLAMA,
        200,
        GENERIC_BODY.as_bytes(),
        &headers,
        &FixedClock(0),
        &FixedIds,
    )
    .unwrap();
    assert_eq!(
        pick(
            &to_json(&reply.additional_headers),
            "llm_provider-x-request-id"
        ),
        body(r#""abc""#)
    );
}
