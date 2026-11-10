#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, BodyDataStream, to_bytes};
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use axum::routing::{get, post};
use futures_util::StreamExt as _;
use rmcp::ServerHandler;
use rmcp::model::{RequestId, ServerConfig};
use rstest::rstest;
use tokio::time::timeout;
use tower::ServiceExt as _;

use crate::mcp::sse_transport::SseTransport;

const DEFAULT_OWNER: &str = "owner-a";
const OTHER_OWNER: &str = "owner-b";
const INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"test-client","version":"1.0"}}}"#
);
const PING: &str = r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const OVERSIZED_BYTES: usize = super::MAX_REQUEST_BYTES + 1;

#[derive(Clone, Copy)]
struct FakeHandler;

impl ServerHandler for FakeHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::default()
    }
}

fn app_with(root_path: &'static str, endpoint: &'static str) -> (Arc<SseTransport>, Router) {
    let transport = Arc::new(SseTransport::new(endpoint));
    let connect = {
        let shared = Arc::clone(&transport);
        move |request: Request<Body>| {
            let manager = Arc::clone(&shared);
            async move { manager.connect(root_path, &owner_of(&request), FakeHandler) }
        }
    };
    let messages = {
        let shared = Arc::clone(&transport);
        move |request: Request<Body>| {
            let manager = Arc::clone(&shared);
            async move {
                let owner = owner_of(&request);
                let (parts, body) = request.into_parts();
                let query = parts.uri.query().map(str::to_owned);
                manager
                    .post(query.as_deref(), &parts.headers, &owner, body)
                    .await
            }
        }
    };
    let router = Router::new()
        .route("/mcp/sse", get(connect))
        .route("/mcp/sse/messages", post(messages));
    (transport, router)
}

fn app() -> (Arc<SseTransport>, Router) {
    app_with("/mcp", "/sse/messages")
}

fn owner_of(request: &Request<Body>) -> String {
    request
        .headers()
        .get("x-test-owner")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(DEFAULT_OWNER)
        .to_owned()
}

fn connect_request() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/mcp/sse")
        .body(Body::empty())
        .expect("a request")
}

fn ping_request(id: u8) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#)
}

fn message_request(target: &str, owner: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(target)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-test-owner", owner)
        .body(Body::from(body.to_owned()))
        .expect("a request")
}

async fn send(router: &Router, request: Request<Body>) -> Response<Body> {
    router.clone().oneshot(request).await.expect("a response")
}

async fn body_text(response: Response<Body>) -> String {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the response body");
    String::from_utf8(bytes.to_vec()).expect("a utf-8 body")
}

async fn next_event(stream: &mut BodyDataStream) -> String {
    let chunk = stream.next().await.expect("an event").expect("an event");
    String::from_utf8(chunk.to_vec()).expect("a utf-8 event")
}

fn endpoint_data(event: &str) -> String {
    let data = event
        .strip_prefix("event: endpoint\r\ndata: ")
        .and_then(|rest| rest.strip_suffix("\r\n\r\n"))
        .expect("the endpoint event");
    data.to_owned()
}

fn message_data(event: &str) -> String {
    let data = event
        .strip_prefix("event: message\r\ndata: ")
        .and_then(|rest| rest.strip_suffix("\r\n\r\n"))
        .expect("a message event");
    data.to_owned()
}

fn session_id_of(uri: &str) -> String {
    let (_, id) = uri
        .split_once("?session_id=")
        .expect("the session id query");
    id.to_owned()
}

fn hyphenated(id: &str) -> String {
    let (first, after_first) = id.split_at(8);
    let (second, after_second) = after_first.split_at(4);
    let (third, after_third) = after_second.split_at(4);
    let (fourth, fifth) = after_third.split_at(4);
    format!("{first}-{second}-{third}-{fourth}-{fifth}")
}

async fn open_stream(router: &Router) -> (BodyDataStream, String) {
    let response = send(router, connect_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let uri = endpoint_data(&next_event(&mut stream).await);
    (stream, uri)
}

#[derive(serde::Deserialize)]
struct InitializeReply {
    id: Option<RequestId>,
    #[serde(default)]
    result: Option<InitializeResult>,
}

#[derive(serde::Deserialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
}

#[derive(serde::Deserialize)]
struct ReplyId {
    id: Option<RequestId>,
}

#[tokio::test]
async fn connect_answers_the_endpoint_event_first() {
    let (_, router) = app();
    let response = send(&router, connect_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        ))
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );
    assert_eq!(
        response.headers().get(header::CONNECTION),
        Some(&HeaderValue::from_static("keep-alive"))
    );
    assert_eq!(
        response.headers().get("x-accel-buffering"),
        Some(&HeaderValue::from_static("no"))
    );
    assert!(response.headers().get("mcp-session-id").is_none());
    let mut stream = response.into_body().into_data_stream();
    let uri = endpoint_data(&next_event(&mut stream).await);
    let id = session_id_of(&uri);
    assert_eq!(uri, format!("/mcp/sse/messages?session_id={id}"));
    assert_eq!(id.len(), 32);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{id}"
    );
}

#[rstest]
#[case::top_level("", "/sse/messages")]
#[case::mounted("/mcp", "/mcp/sse/messages")]
#[case::trailing_slash("/mcp/", "/mcp/sse/messages")]
#[case::many_trailing_slashes("/mcp///", "/mcp/sse/messages")]
#[case::quoted_space("/mc p", "/mc%20p/sse/messages")]
#[case::quoted_non_ascii("/m\u{e7}p", "/m%C3%A7p/sse/messages")]
#[tokio::test]
async fn connect_builds_the_message_path_from_the_root_path(
    #[case] root_path: &'static str,
    #[case] expected_path: &str,
) {
    let (_, router) = app_with(root_path, "/sse/messages");
    let (_, uri) = open_stream(&router).await;
    let (path, _) = uri
        .split_once("?session_id=")
        .expect("the session id query");
    assert_eq!(path, expected_path);
}

#[tokio::test]
async fn new_adds_a_leading_slash_to_a_relative_endpoint() {
    let (_, router) = app_with("", "sse/messages");
    let (_, uri) = open_stream(&router).await;
    let path = uri
        .split_once("?session_id=")
        .expect("the session id query")
        .0;
    assert_eq!(path, "/sse/messages");
}

#[tokio::test]
async fn new_accepts_an_endpoint_with_a_scheme() {
    let (_, router) = app_with("", "http://example.com/sse");
    let (_, uri) = open_stream(&router).await;
    let path = uri
        .split_once("?session_id=")
        .expect("the session id query")
        .0;
    assert_eq!(path, "/http%3A//example.com/sse");
}

#[tokio::test]
async fn relays_the_services_messages_to_the_client() {
    let (_, router) = app();
    let (mut stream, uri) = open_stream(&router).await;
    let accepted = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert_eq!(body_text(accepted).await, "Accepted");
    let reply: InitializeReply =
        serde_json::from_str(&message_data(&next_event(&mut stream).await))
            .expect("the initialize reply");
    assert_eq!(reply.id, Some(RequestId::Number(1)));
    assert_eq!(
        reply.result.expect("an initialize result").protocol_version,
        "2025-06-18"
    );
    let accepted_ping = send(&router, message_request(&uri, DEFAULT_OWNER, PING)).await;
    assert_eq!(accepted_ping.status(), StatusCode::ACCEPTED);
    assert_eq!(
        next_event(&mut stream).await,
        "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\r\n\r\n"
    );
}

#[tokio::test]
async fn drops_the_session_when_the_client_leaves() {
    let (_, router) = app();
    let (stream, uri) = open_stream(&router).await;
    drop(stream);
    let refused = send(&router, message_request(&uri, DEFAULT_OWNER, PING)).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(refused).await, "Could not find session");
}

#[tokio::test]
async fn ends_the_stream_and_the_session_when_the_service_stops() {
    let (_, router) = app();
    let (mut stream, uri) = open_stream(&router).await;
    let early = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZED)).await;
    assert_eq!(early.status(), StatusCode::ACCEPTED);
    timeout(Duration::from_secs(5), async {
        while stream.next().await.is_some() {}
    })
    .await
    .expect("the stopped service to end the stream");
    let refused = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(refused).await, "Could not find session");
}

#[tokio::test(start_paused = true)]
async fn sends_a_ping_comment_on_the_sse_starlette_interval() {
    let (_, router) = app();
    let (mut stream, _) = open_stream(&router).await;
    let ping = next_event(&mut stream).await;
    assert!(ping.starts_with(": ping - "), "{ping}");
    assert!(ping.ends_with("\r\n\r\n"), "{ping}");
}

#[rstest]
#[case::absent(None)]
#[case::text(Some("text/plain"))]
#[case::html(Some("text/html; charset=utf-8"))]
#[tokio::test]
async fn rejects_a_message_without_a_json_content_type(#[case] content_type: Option<&str>) {
    let (_, router) = app();
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/mcp/sse/messages?session_id=bad");
    if let Some(value) = content_type {
        builder = builder.header(header::CONTENT_TYPE, value);
    }
    let request = builder.body(Body::from(PING)).expect("a request");
    let refused = send(&router, request).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Invalid Content-Type header");
}

#[tokio::test]
async fn checks_the_content_type_before_the_session() {
    let (_, router) = app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/mcp/sse/messages")
        .body(Body::from(PING))
        .expect("a request");
    let refused = send(&router, request).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Invalid Content-Type header");
}

#[tokio::test]
async fn accepts_a_json_content_type_with_parameters() {
    let (_, router) = app();
    let (_stream, uri) = open_stream(&router).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri.as_str())
        .header(header::CONTENT_TYPE, "Application/JSON; charset=utf-8")
        .header("x-test-owner", DEFAULT_OWNER)
        .body(Body::from(INITIALIZE))
        .expect("a request");
    let accepted = send(&router, request).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
}

#[rstest]
#[case::no_query("/mcp/sse/messages")]
#[case::other_parameter("/mcp/sse/messages?foo=1")]
#[tokio::test]
async fn rejects_a_message_without_a_session_id(#[case] target: &str) {
    let (_, router) = app();
    let refused = send(&router, message_request(target, DEFAULT_OWNER, PING)).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "session_id is required");
}

#[rstest]
#[case::not_a_uuid("/mcp/sse/messages?session_id=not-a-uuid")]
#[case::empty("/mcp/sse/messages?session_id=")]
#[case::blank("/mcp/sse/messages?session_id")]
#[case::too_short("/mcp/sse/messages?session_id=0123456789abcdef")]
#[case::non_hex("/mcp/sse/messages?session_id=zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz")]
#[tokio::test]
async fn rejects_a_session_id_that_is_not_a_hex_uuid(#[case] target: &str) {
    let (_, router) = app();
    let refused = send(&router, message_request(target, DEFAULT_OWNER, PING)).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Invalid session ID");
}

#[tokio::test]
async fn checks_the_session_before_the_body() {
    let (_, router) = app();
    let missing = send(
        &router,
        message_request("/mcp/sse/messages", DEFAULT_OWNER, "not json"),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(missing).await, "session_id is required");
    let malformed = send(
        &router,
        message_request(
            "/mcp/sse/messages?session_id=bad",
            DEFAULT_OWNER,
            "not json",
        ),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(malformed).await, "Invalid session ID");
    let unknown = send(
        &router,
        message_request(
            "/mcp/sse/messages?session_id=ffffffffffffffffffffffffffffffff",
            DEFAULT_OWNER,
            "not json",
        ),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(unknown).await, "Could not find session");
}

#[tokio::test]
async fn rejects_an_unknown_session() {
    let (_, router) = app();
    let target = "/mcp/sse/messages?session_id=ffffffffffffffffffffffffffffffff";
    let refused = send(&router, message_request(target, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(refused).await, "Could not find session");
}

#[tokio::test]
async fn rejects_a_message_from_another_owner() {
    let (_, router) = app();
    let (mut stream, uri) = open_stream(&router).await;
    let refused = send(&router, message_request(&uri, OTHER_OWNER, INITIALIZE)).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_text(refused).await, "Could not find session");
    let accepted = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let reply: InitializeReply =
        serde_json::from_str(&message_data(&next_event(&mut stream).await))
            .expect("the initialize reply");
    assert_eq!(reply.id, Some(RequestId::Number(1)));
}

#[rstest]
#[case::not_json("not json")]
#[case::not_jsonrpc(r#"{"hello":"world"}"#)]
#[tokio::test]
async fn rejects_an_undecodable_message_and_keeps_the_session(#[case] body: &str) {
    let (_, router) = app();
    let (mut stream, uri) = open_stream(&router).await;
    let refused = send(&router, message_request(&uri, DEFAULT_OWNER, body)).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Could not parse message");
    let accepted = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let reply: InitializeReply =
        serde_json::from_str(&message_data(&next_event(&mut stream).await))
            .expect("the initialize reply");
    assert_eq!(reply.id, Some(RequestId::Number(1)));
}

#[tokio::test]
async fn takes_the_last_session_id_when_the_query_repeats_it() {
    let (_, router) = app();
    let (_stream, uri) = open_stream(&router).await;
    let id = session_id_of(&uri);
    let accepted_target = format!("/mcp/sse/messages?session_id=bad&session_id={id}");
    let accepted = send(
        &router,
        message_request(&accepted_target, DEFAULT_OWNER, INITIALIZE),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let refused_target = format!("/mcp/sse/messages?session_id={id}&session_id=bad");
    let refused = send(
        &router,
        message_request(&refused_target, DEFAULT_OWNER, INITIALIZE),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Invalid session ID");
}

#[tokio::test]
async fn matches_the_session_id_in_any_standard_uuid_spelling() {
    let (_, router) = app();
    let (mut stream, uri) = open_stream(&router).await;
    let id = session_id_of(&uri);
    let accepted = send(&router, message_request(&uri, DEFAULT_OWNER, INITIALIZE)).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let uppercase_target = format!("/mcp/sse/messages?session_id={}", id.to_ascii_uppercase());
    let accepted_uppercase = send(
        &router,
        message_request(&uppercase_target, DEFAULT_OWNER, &ping_request(2)),
    )
    .await;
    assert_eq!(accepted_uppercase.status(), StatusCode::ACCEPTED);
    let hyphenated_target = format!("/mcp/sse/messages?session_id={}", hyphenated(&id));
    let accepted_hyphenated = send(
        &router,
        message_request(&hyphenated_target, DEFAULT_OWNER, &ping_request(3)),
    )
    .await;
    assert_eq!(accepted_hyphenated.status(), StatusCode::ACCEPTED);
    let braced_target = format!("/mcp/sse/messages?session_id=%7B{id}%7D");
    let accepted_braced = send(
        &router,
        message_request(&braced_target, DEFAULT_OWNER, &ping_request(4)),
    )
    .await;
    assert_eq!(accepted_braced.status(), StatusCode::ACCEPTED);
    let urn_target = format!("/mcp/sse/messages?session_id=urn:uuid:{id}");
    let accepted_urn = send(
        &router,
        message_request(&urn_target, DEFAULT_OWNER, &ping_request(5)),
    )
    .await;
    assert_eq!(accepted_urn.status(), StatusCode::ACCEPTED);
    let initialize: InitializeReply =
        serde_json::from_str(&message_data(&next_event(&mut stream).await))
            .expect("the initialize reply");
    assert_eq!(initialize.id, Some(RequestId::Number(1)));
    let ping: ReplyId = serde_json::from_str(&message_data(&next_event(&mut stream).await))
        .expect("the first ping reply");
    assert_eq!(ping.id, Some(RequestId::Number(2)));
}

#[tokio::test]
async fn rejects_a_body_over_the_request_limit() {
    let (_, router) = app();
    let (_stream, uri) = open_stream(&router).await;
    let body = "x".repeat(OVERSIZED_BYTES);
    let refused = send(&router, message_request(&uri, DEFAULT_OWNER, &body)).await;
    assert_eq!(refused.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body_text(refused).await, "Request body too large");
}

#[tokio::test]
async fn refuses_an_oversized_body_after_the_session_checks() {
    let (_, router) = app();
    let body = "x".repeat(OVERSIZED_BYTES);
    let refused = send(
        &router,
        message_request("/mcp/sse/messages?session_id=bad", DEFAULT_OWNER, &body),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Invalid session ID");
}

#[tokio::test]
async fn reads_a_body_at_the_request_limit() {
    let (_, router) = app();
    let (_stream, uri) = open_stream(&router).await;
    let body = "x".repeat(super::MAX_REQUEST_BYTES);
    let refused = send(&router, message_request(&uri, DEFAULT_OWNER, &body)).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(refused).await, "Could not parse message");
}
