use betterlite_http::proxy::UpstreamResponse;
use http::{HeaderMap, HeaderValue};
use uuid::Uuid;

use crate::api::copilot_chat::error::CopilotChatError;
use crate::api::copilot_chat::native_reply::{NativeReply, transform_parsed_response};
use crate::api::copilot_chat::reply::transform_response;

const NATIVE_TOOL_REPLY: &str = r#"{"id":"msg_1","stop_reason":"tool_use","content":[
    {"type":"text","text":"calling"},
    {"type":"tool_use","id":"toolu_1","name":"lookup","input":{"q":"x"}}],
    "usage":{"input_tokens":3,"output_tokens":4}}"#;

fn identifier() -> Uuid {
    Uuid::from_u128(1)
}

fn upstream(status: u16, body: &str) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    headers.insert("x-copilot-trace", HeaderValue::from_static("abc"));
    UpstreamResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn a_repaired_reply_keeps_its_status_and_headers_and_decodes_its_tool_call() {
    let reply =
        transform_response(upstream(202, NATIVE_TOOL_REPLY), None, false, identifier).unwrap();
    assert_eq!(reply.response.status, 202);
    assert_eq!(reply.response.headers["x-copilot-trace"], "abc");
    let [choice] = reply.choices.as_slice() else {
        panic!("expected one choice, got {:?}", reply.choices);
    };
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason, "tool_calls");
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("calling"));
    let calls = choice.message.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "toolu_1");
    let function = calls[0].function.as_ref().unwrap();
    assert_eq!(function.name.as_deref(), Some("lookup"));
    assert_eq!(function.arguments, r#"{"q": "x"}"#);
}

#[test]
fn the_rewritten_body_is_the_parsed_repair_of_the_same_reply() {
    let reply =
        transform_response(upstream(200, NATIVE_TOOL_REPLY), None, false, identifier).unwrap();
    let parsed: NativeReply = serde_json::from_str(NATIVE_TOOL_REPLY).unwrap();
    let repaired = transform_parsed_response(parsed).unwrap();
    assert_eq!(reply.response.body, serde_json::to_vec(&repaired).unwrap());
}

#[test]
fn a_reply_with_choices_reaches_the_decoder_byte_for_byte() {
    let body = r#"{"usage":{"input_tokens":1},"choices":[{"index":2,"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}]}"#;
    let reply = transform_response(upstream(200, body), None, false, identifier).unwrap();
    assert_eq!(reply.response.body, body.as_bytes());
    assert_eq!(reply.choices[0].index, 2);
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("hi"));
}

#[test]
fn an_empty_probe_reply_decodes_to_an_empty_string_with_length() {
    let reply =
        transform_response(upstream(200, r#"{"content":[]}"#), None, false, identifier).unwrap();
    assert_eq!(reply.choices[0].finish_reason, "length");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some(""));
    assert!(reply.choices[0].message.tool_calls.is_none());
}

#[test]
fn a_tool_only_reply_decodes_to_null_content() {
    let body = r#"{"content":[{"type":"tool_use","id":"t","name":"f","input":{}}]}"#;
    let reply = transform_response(upstream(200, body), None, false, identifier).unwrap();
    assert_eq!(reply.choices[0].message.content, None);
    assert_eq!(reply.choices[0].finish_reason, "tool_calls");
}

#[rstest::rstest]
#[case::not_json("upstream exploded")]
#[case::top_level_array("[1,2]")]
#[case::choices_of_the_wrong_type(r#"{"choices":"none"}"#)]
fn a_reply_the_decoder_rejects_fails_with_the_original_response(#[case] body: &str) {
    let error = transform_response(upstream(502, body), None, false, identifier).unwrap_err();
    let CopilotChatError::Parent { response, .. } = error else {
        panic!("expected the parent error path, got {error:?}");
    };
    assert_eq!(response.status, 502);
    assert_eq!(response.headers["x-copilot-trace"], "abc");
    assert_eq!(response.body, body.as_bytes());
}

#[test]
fn a_repair_failure_is_reported_without_the_response() {
    let body = r#"{"content":"x","usage":{"prompt_tokens":"many"}}"#;
    let error = transform_response(upstream(200, body), None, false, identifier).unwrap_err();
    assert!(matches!(
        error,
        CopilotChatError::Usage {
            name: "prompt_tokens"
        }
    ));
}
