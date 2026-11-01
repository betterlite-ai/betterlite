#![cfg(test)]

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::envelope::{self, IdValue, MethodValue, ParamsValue};

fn fields_of(text: &str) -> Fields {
    match Json::parse(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn id_json(id: &IdValue) -> String {
    serde_json::to_string(id).unwrap()
}

fn method_json(method: &MethodValue) -> String {
    serde_json::to_string(method).unwrap()
}

#[test]
fn request_from_call_data_builds_tools_call_envelope() {
    let data = fields_of(
        r#"{"mcp_tool_name": "search", "arguments": {"q": "x"}, "litellm_call_id": "abc"}"#,
    );
    let request = envelope::build_request_payload(&data).expect("built request");
    assert_eq!(request.jsonrpc, "2.0");
    assert_eq!(id_json(&request.id), r#""abc""#);
    assert_eq!(method_json(&request.method), r#""tools/call""#);
    assert_eq!(
        serde_json::to_string(&request.params).unwrap(),
        r#"{"name":"search","arguments":{"q": "x"}}"#
    );
}

#[test]
fn request_from_call_data_defaults_id_and_empty_arguments() {
    let data = fields_of(r#"{"tool_name": "search"}"#);
    let request = envelope::build_request_payload(&data).expect("built request");
    assert_eq!(id_json(&request.id), r#""litellm-mcp""#);
    assert_eq!(
        serde_json::to_string(&request.params).unwrap(),
        r#"{"name":"search","arguments":{}}"#
    );
}

#[test]
fn request_from_call_data_ignores_non_object_arguments() {
    let data = fields_of(r#"{"name": "search", "arguments": "not an object"}"#);
    let request = envelope::build_request_payload(&data).expect("built request");
    assert_eq!(
        serde_json::to_string(&request.params).unwrap(),
        r#"{"name":"search","arguments":{}}"#
    );
}

#[test]
fn request_from_call_data_requires_a_name() {
    let data = fields_of(r#"{"arguments": {}}"#);
    assert!(envelope::build_request_payload(&data).is_none());
}

#[test]
fn request_from_jsonrpc_preserves_given_fields() {
    let data = fields_of(
        r#"{"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "search"}}"#,
    );
    let request = envelope::build_request_payload(&data).expect("built request");
    assert_eq!(id_json(&request.id), "5");
    assert_eq!(
        serde_json::to_string(&request.params).unwrap(),
        r#"{"name": "search"}"#
    );
}

#[test]
fn request_from_jsonrpc_fills_defaults_for_missing_fields() {
    let data = fields_of(r#"{"jsonrpc": "2.0"}"#);
    let request = envelope::build_request_payload(&data).expect("built request");
    assert_eq!(id_json(&request.id), r#""litellm-mcp""#);
    assert_eq!(method_json(&request.method), r#""tools/call""#);
    assert!(matches!(request.params, ParamsValue::Empty(_)));
}

#[test]
fn response_payload_carries_result_and_falls_back_to_request_id() {
    let request_data = fields_of(r#"{"mcp_tool_name": "search", "litellm_call_id": "abc"}"#);
    let response = Json::parse(r#"{"content": [{"type": "text", "text": "hi"}]}"#);
    let payload =
        envelope::build_response_payload(&request_data, &response).expect("built response");
    assert_eq!(id_json(&payload.id), r#""abc""#);
    assert!(payload.result.is_some());
    assert!(payload.error.is_none());
}

#[test]
fn response_payload_prefers_id_override_from_response() {
    let request_data = fields_of(r#"{"mcp_tool_name": "search"}"#);
    let response = Json::parse(r#"{"jsonrpc": "2.0", "id": 42, "result": {"content": []}}"#);
    let payload =
        envelope::build_response_payload(&request_data, &response).expect("built response");
    assert_eq!(id_json(&payload.id), "42");
}

#[test]
fn response_payload_none_when_response_is_unshaped() {
    let request_data = fields_of(r#"{"mcp_tool_name": "search"}"#);
    let response = Json::parse("null");
    assert!(envelope::build_response_payload(&request_data, &response).is_none());
}
