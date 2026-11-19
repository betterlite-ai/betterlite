#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use serde_json::Value;

use super::{McpReplyError, correlate, correlation};
use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

const PATH: &str = "/__mcp_fixture/alpha";
const MARKER: &str = "$mcp_request_id";
const NOTIFICATION: &str = r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#;

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

fn answered(body: Value) -> UpstreamAnswer {
    scripted(200, Some(body), None)
}

fn request_body(id: &str, method: &str) -> Value {
    json_body(&format!(
        r#"{{"jsonrpc": "2.0", "id": {id}, "method": "{method}"}}"#
    ))
}

fn answer_body(id: &str, fields: &str) -> Value {
    json_body(&format!(r#"{{"jsonrpc": "2.0", "id": {id}, {fields}}}"#))
}

fn marked_body(fields: &str) -> Value {
    answer_body(&format!(r#""{MARKER}""#), fields)
}

fn marked_answer() -> UpstreamAnswer {
    answered(marked_body(r#""result": {}"#))
}

fn assert_marker_without_id(result: Result<(), McpReplyError>) {
    let error = result.unwrap_err();
    assert!(matches!(&error, McpReplyError::MarkerWithoutId { .. }));
    assert!(error.to_string().contains(PATH));
}

#[test]
fn integer_id() {
    let request = request_body("7", "tools/list");
    let request_correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.body, Some(answer_body("7", r#""result": {}"#)));
    let large = request_body("18446744073709551615", "tools/list");
    let mut large_reply = marked_answer();
    correlate(&mut large_reply, &correlation(PATH, &large)).unwrap();
    assert_eq!(
        large_reply.body,
        Some(answer_body("18446744073709551615", r#""result": {}"#))
    );
}

#[test]
fn string_id() {
    let request = request_body(r#""req-a""#, "tools/list");
    let request_correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(
        reply.body,
        Some(answer_body(r#""req-a""#, r#""result": {}"#))
    );
}

#[test]
fn no_marker() {
    let request = request_body("7", "tools/list");
    let request_correlation = correlation(PATH, &request);
    let mut reply = answered(answer_body("99", r#""result": {}"#));
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.body, Some(answer_body("99", r#""result": {}"#)));
    let mut literal = answered(answer_body(r#""99""#, r#""result": {}"#));
    correlate(&mut literal, &request_correlation).unwrap();
    assert_eq!(
        literal.body,
        Some(answer_body(r#""99""#, r#""result": {}"#))
    );
}

#[test]
fn wrong_path() {
    let request = request_body("7", "chat");
    let request_correlation = correlation("/v1/chat/completions", &request);
    let mut reply = marked_answer();
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.body, Some(marked_body(r#""result": {}"#)));
    let near_miss = correlation("/__mcp_fixture", &request);
    let mut near_miss_reply = marked_answer();
    correlate(&mut near_miss_reply, &near_miss).unwrap();
    assert_eq!(near_miss_reply.body, Some(marked_body(r#""result": {}"#)));
}

#[test]
fn nested_marker_unchanged() {
    let request = request_body("7", "tools/call");
    let request_correlation = correlation(PATH, &request);
    let content = format!(r#""result": {{"content": [{{"type": "text", "text": "{MARKER}"}}]}}"#);
    let mut reply = answered(marked_body(&content));
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.body, Some(answer_body("7", &content)));
}

#[test]
fn missing_id() {
    let request = json_body(NOTIFICATION);
    let request_correlation = correlation(PATH, &request);
    let mut reply = marked_answer();
    let error = correlate(&mut reply, &request_correlation).unwrap_err();
    assert!(matches!(&error, McpReplyError::MarkerWithoutId { .. }));
    let message = error.to_string();
    assert!(message.contains(PATH));
    assert!(message.contains(MARKER));
}

fn assert_unusable_request(request: &Value) {
    let mut reply = marked_answer();
    assert_marker_without_id(correlate(&mut reply, &correlation(PATH, request)));
}

#[test]
fn invalid_id() {
    assert_unusable_request(&request_body("1.5", "tools/list"));
    assert_unusable_request(&request_body("true", "tools/list"));
    assert_unusable_request(&request_body("null", "tools/list"));
    assert_unusable_request(&request_body("[7]", "tools/list"));
    assert_unusable_request(&request_body(r#"{"value": 7}"#, "tools/list"));
}

#[test]
fn unusable_body_gives_no_id() {
    assert_unusable_request(&json_body(
        r#"[{"jsonrpc": "2.0", "id": 7, "method": "tools/list"}]"#,
    ));
    assert_unusable_request(&Value::String("not json".to_owned()));
}

#[test]
fn jsonrpc_not_2_0() {
    let request = request_body("7", "tools/list");
    let request_correlation = correlation(PATH, &request);
    let mut missing = answered(json_body(&format!(
        r#"{{"id": "{MARKER}", "result": {{}}}}"#
    )));
    correlate(&mut missing, &request_correlation).unwrap();
    assert_eq!(
        missing.body,
        Some(json_body(&format!(
            r#"{{"id": "{MARKER}", "result": {{}}}}"#
        )))
    );
    let mut other = answered(json_body(&format!(
        r#"{{"jsonrpc": "1.0", "id": "{MARKER}", "result": {{}}}}"#
    )));
    correlate(&mut other, &request_correlation).unwrap();
    assert_eq!(
        other.body,
        Some(json_body(&format!(
            r#"{{"jsonrpc": "1.0", "id": "{MARKER}", "result": {{}}}}"#
        )))
    );
}

#[test]
fn non_object_answer_unchanged() {
    let request = request_body("7", "tools/list");
    let request_correlation = correlation(PATH, &request);
    let mut text = answered(Value::String(MARKER.to_owned()));
    correlate(&mut text, &request_correlation).unwrap();
    assert_eq!(text.body, Some(Value::String(MARKER.to_owned())));
    let array = format!(r#"["{MARKER}"]"#);
    let mut array_reply = answered(json_body(&array));
    correlate(&mut array_reply, &request_correlation).unwrap();
    assert_eq!(array_reply.body, Some(json_body(&array)));
}

#[test]
fn notification_without_marker() {
    let request = json_body(NOTIFICATION);
    let request_correlation = correlation(PATH, &request);
    let mut reply = scripted(202, None, Some(Vec::new()));
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.status, 202);
    assert_eq!(reply.body, None);
    assert_eq!(reply.events, Some(Vec::new()));
}

#[test]
fn error_reply() {
    let request = request_body(r#""req-a""#, "tools/call");
    let request_correlation = correlation(PATH, &request);
    let error = r#""error": {"code": -32601, "message": "method not found"}"#;
    let mut reply = answered(marked_body(error));
    correlate(&mut reply, &request_correlation).unwrap();
    assert_eq!(reply.body, Some(answer_body(r#""req-a""#, error)));
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

fn session_replies() -> [UpstreamAnswer; 3] {
    [
        answered(marked_body(
            r#""result": {"protocolVersion": "2025-03-26", "capabilities": {}, "serverInfo": {"name": "test-upstream", "version": "1"}}"#,
        )),
        scripted(202, None, Some(Vec::new())),
        answered(marked_body(r#""result": {"tools": []}"#)),
    ]
}

#[test]
fn loopback_session_correlates_each_reply() {
    let replies = session_replies();
    let mock = MockProvider::start(MockScript::try_from(&replies[..]).unwrap()).unwrap();
    let initialize =
        json_body(r#"{"jsonrpc": "2.0", "id": 41, "method": "initialize", "params": {}}"#);
    let notification = json_body(NOTIFICATION);
    let list = request_body(r#""list-2""#, "tools/list");

    let initialized = respond(&mock, PATH, &initialize);
    let notified = respond(&mock, PATH, &notification);
    let listed = respond(&mock, PATH, &list);

    assert_eq!(status_of(&initialized), 200);
    assert_eq!(
        json_body(response_body(&initialized)).get("id"),
        Some(&json_body("41"))
    );
    assert_eq!(status_of(&notified), 202);
    assert_eq!(response_body(&notified), "");
    assert_eq!(status_of(&listed), 200);
    assert_eq!(
        json_body(response_body(&listed)).get("id"),
        Some(&json_body(r#""list-2""#))
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
    let replies = [marked_answer()];
    let mock = MockProvider::start(MockScript::try_from(&replies[..]).unwrap()).unwrap();
    let notification = json_body(NOTIFICATION);

    let failed = respond(&mock, PATH, &notification);

    assert_eq!(status_of(&failed), 500);
    let (_calls, failure) = mock.finish_recording().unwrap();
    let recorded = failure.unwrap();
    let message = recorded.to_string();
    assert!(message.contains(PATH));
    assert!(message.contains(MARKER));
}
