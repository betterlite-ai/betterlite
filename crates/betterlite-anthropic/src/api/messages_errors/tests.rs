#![cfg(test)]

use super::*;
use rstest::rstest;
use serde_json::{Value, json};

#[test]
fn status_error_types() {
    let response = transform_messages_error(429, "broken", None);
    assert_eq!(response.error.error_type, json!("rate_limit_error"));
    assert_eq!(response.error.message, json!("broken"));
    assert!(response.request_id.is_none());
}

#[test]
fn provider_message_precedence() {
    let response = transform_messages_error(
        500,
        r#"{"detail":{"message":"inner"},"Message":"aws","message":"generic"}"#,
        None,
    );
    assert_eq!(response.error.message, json!("inner"));
    let response = transform_messages_error(500, r#"{"detail":{},"Message":"aws"}"#, None);
    assert_eq!(
        response.error.message,
        json!(r#"{"detail":{},"Message":"aws"}"#)
    );
}

#[test]
fn anthropic_error_passthrough() {
    let response = transform_messages_error(
        401,
        r#"{"type":"error","error":{"type":null,"message":"bad","code":"provider"}}"#,
        Some("req_new"),
    );
    assert_eq!(response.error.error_type, Value::Null);
    assert_eq!(response.error.extra.get("code"), Some(&json!("provider")));
    assert_eq!(response.request_id, Some(json!("req_new")));
}

#[test]
fn status_bearing_error_frame() {
    let frame = anthropic_error_sse_frame(429, "slow");
    assert_eq!(frame.status_code, 429);
    assert!(frame.body().starts_with("event: error\ndata: "));
    assert_eq!(
        frame.json_body(Some("call_a")).error.litellm_call_id,
        Some("call_a".to_owned())
    );
}

#[test]
fn response_optional_fields() {
    let bytes = br#"{"type":"message","content":[{"type":"text","text":"hi","citations":null},{"type":"tool_use","id":"t","input":{},"name":"f"},{"type":"thinking","thinking":"x","signature":"s"},{"type":"redacted_thinking","data":"d"},{"type":"compaction","foo":7}],"stop_reason":"compaction","usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":3}}"#;
    let decoded = decode_messages_errors(
        bytes,
        WireDecodeLimits {
            max_json_bytes: 2048,
            max_event_bytes: 2048,
            max_items: 100,
        },
    );
    assert!(matches!(decoded, Ok(DecodedMessagesErrors::Response(_))));
}

#[test]
fn wire_limits_reject_before_parse() {
    let error = decode_messages_errors(
        br#"{"type":"message"}"#,
        WireDecodeLimits {
            max_json_bytes: 1,
            max_event_bytes: 100,
            max_items: 100,
        },
    );
    assert!(matches!(error, Err(WireDecodeError::TooLarge)));
}

#[rstest]
#[case::invalid_json(&include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/messages_errors/invalid-json"))[..])]
#[case::invalid_utf8(&include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/messages_errors/invalid-utf8"))[..])]
fn replays_corpus(#[case] bytes: &[u8]) {
    let result = decode_messages_errors(
        bytes,
        WireDecodeLimits {
            max_json_bytes: 4096,
            max_event_bytes: 4096,
            max_items: 100,
        },
    );
    assert!(result.is_err());
}
