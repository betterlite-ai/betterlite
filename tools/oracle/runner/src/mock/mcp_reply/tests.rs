#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use serde_json::{Value, json};

use super::{McpReplyError, correlate, correlation};
use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

const PATH: &str = "/__mcp_fixture/alpha";
const MARKER: &str = "$mcp_request_id";

fn json_body(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn scripted(status: u16, body: Option<Value>, events: Option<Vec<String>>) -> UpstreamAnswer {
    UpstreamAnswer {
        status,
        headers: BTreeMap::new(),
        body,
        events,
    }
}

fn marked_answer() -> UpstreamAnswer {
    scripted(
        200,
        Some(json!({"jsonrpc": "2.0", "id": MARKER, "result": {}})),
        None,
    )
}

fn assert_marker_without_id(result: Result<(), McpReplyError>) {
    let error = result.unwrap_err();
    assert!(matches!(&error, McpReplyError::MarkerWithoutId { .. }));
    assert!(error.to_string().contains(PATH));
}

#[test]
fn integer_id() {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"});
    let correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({"jsonrpc": "2.0", "id": 7, "result": {}}))
    );
}

#[test]
fn string_id() {
    let request = json!({"jsonrpc": "2.0", "id": "req-a", "method": "tools/list"});
    let correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({"jsonrpc": "2.0", "id": "req-a", "result": {}}))
    );
}

#[test]
fn no_marker() {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"});
    let correlation = correlation(PATH, &request);
    let mut reply = scripted(
        200,
        Some(json!({"jsonrpc": "2.0", "id": 99, "result": {}})),
        None,
    );
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({"jsonrpc": "2.0", "id": 99, "result": {}}))
    );
}

#[test]
fn wrong_path() {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": "chat"});
    let correlation = correlation("/v1/chat/completions", &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({"jsonrpc": "2.0", "id": MARKER, "result": {}}))
    );
}

#[test]
fn nested_marker_unchanged() {
    let request = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call"});
    let correlation = correlation(PATH, &request);
    let mut reply = scripted(
        200,
        Some(json!({
            "jsonrpc": "2.0",
            "id": MARKER,
            "result": {"content": [{"type": "text", "text": MARKER}]}
        })),
        None,
    );
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {"content": [{"type": "text", "text": MARKER}]}
        }))
    );
}

#[test]
fn missing_id() {
    let request = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    let error = correlate(&mut reply, &correlation).unwrap_err();
    assert!(matches!(&error, McpReplyError::MarkerWithoutId { .. }));
    let message = error.to_string();
    assert!(message.contains(PATH));
    assert!(message.contains(MARKER));
}

#[test]
fn invalid_id() {
    let float = json!({"jsonrpc": "2.0", "id": 1.5, "method": "tools/list"});
    let mut float_reply = marked_answer();
    assert_marker_without_id(correlate(&mut float_reply, &correlation(PATH, &float)));
    let boolean = json!({"jsonrpc": "2.0", "id": true, "method": "tools/list"});
    let mut boolean_reply = marked_answer();
    assert_marker_without_id(correlate(&mut boolean_reply, &correlation(PATH, &boolean)));
}

#[test]
fn notification_without_marker() {
    let request = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let correlation = correlation(PATH, &request);
    let mut reply = scripted(202, None, Some(Vec::new()));
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(reply.status, 202);
    assert_eq!(reply.body, None);
    assert_eq!(reply.events, Some(Vec::new()));
}

#[test]
fn error_reply() {
    let request = json!({"jsonrpc": "2.0", "id": "req-a", "method": "tools/call"});
    let correlation = correlation(PATH, &request);
    let mut reply = scripted(
        200,
        Some(json!({
            "jsonrpc": "2.0",
            "id": MARKER,
            "error": {"code": -32601, "message": "method not found"}
        })),
        None,
    );
    correlate(&mut reply, &correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(json!({
            "jsonrpc": "2.0",
            "id": "req-a",
            "error": {"code": -32601, "message": "method not found"}
        }))
    );
}

fn respond(mock: &MockProvider, path: &str, body: &Value) -> String {
    let encoded = serde_json::to_string(body).unwrap();
    let address = mock.base_url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{encoded}",
        encoded.len()
    )
    .unwrap();
    let mut received = String::new();
    stream.read_to_string(&mut received).unwrap();
    received
}

fn status_of(response: &str) -> u16 {
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn response_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

#[test]
fn loopback_session_correlates_each_reply() {
    let replies = [
        scripted(
            200,
            Some(json!({
                "jsonrpc": "2.0",
                "id": MARKER,
                "result": {"protocolVersion": "2025-03-26", "capabilities": {}, "serverInfo": {"name": "test-upstream", "version": "1"}}
            })),
            None,
        ),
        scripted(202, None, Some(Vec::new())),
        scripted(
            200,
            Some(json!({"jsonrpc": "2.0", "id": MARKER, "result": {"tools": []}})),
            None,
        ),
    ];
    let mock = MockProvider::start(MockScript::try_from(&replies[..]).unwrap()).unwrap();
    let initialize = json!({"jsonrpc": "2.0", "id": 41, "method": "initialize", "params": {}});
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let list = json!({"jsonrpc": "2.0", "id": "list-2", "method": "tools/list"});

    let initialized = respond(&mock, PATH, &initialize);
    let notified = respond(&mock, PATH, &notification);
    let listed = respond(&mock, PATH, &list);

    assert_eq!(status_of(&initialized), 200);
    assert_eq!(
        json_body(response_body(&initialized)).get("id"),
        Some(&json!(41))
    );
    assert_eq!(status_of(&notified), 202);
    assert_eq!(response_body(&notified), "");
    assert_eq!(status_of(&listed), 200);
    assert_eq!(
        json_body(response_body(&listed)).get("id"),
        Some(&json!("list-2"))
    );

    let trace = mock.finish().unwrap();
    assert_eq!(trace.len(), 3);
    assert_eq!(trace[0].request.path, PATH);
    assert_eq!(trace[1].request.path, PATH);
    assert_eq!(trace[2].request.path, PATH);
    assert_eq!(trace[0].request.body, initialize);
    assert_eq!(trace[1].request.body, notification);
    assert_eq!(trace[2].request.body, list);
}

#[test]
fn loopback_marker_without_id_fails() {
    let replies = [scripted(
        200,
        Some(json!({"jsonrpc": "2.0", "id": MARKER, "result": {}})),
        None,
    )];
    let mock = MockProvider::start(MockScript::try_from(&replies[..]).unwrap()).unwrap();
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});

    let failed = respond(&mock, PATH, &notification);

    assert_eq!(status_of(&failed), 500);
    let (_calls, failure) = mock.finish_recording().unwrap();
    let failure = failure.unwrap();
    let message = failure.to_string();
    assert!(message.contains(PATH));
    assert!(message.contains(MARKER));
}
