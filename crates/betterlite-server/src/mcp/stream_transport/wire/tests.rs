#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode, header};
use axum::routing::any;
use futures_util::StreamExt as _;
use rmcp::ErrorData;
use rmcp::RoleServer;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, RequestId, ServerConfig,
    ServerJsonRpcMessage, ServerResult,
};
use rmcp::service::RequestContext;
use tower::ServiceExt as _;

use super::{decode_message, sse_event};
use crate::mcp::stream_transport::StreamTransport;

#[derive(Clone, Copy)]
struct FakeHandler;

impl ServerHandler for FakeHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::default()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if let Some(token) = context.meta.get_progress_token() {
            let _sent = context
                .peer
                .notify_progress(ProgressNotificationParam::new(token, 1.0))
                .await;
        }
        Ok(ListToolsResult::default())
    }
}

fn app(shared: Arc<StreamTransport>) -> Router {
    Router::new().route(
        "/mcp",
        any(move |request: Request<Body>| {
            let transport = Arc::clone(&shared);
            async move { transport.handle(request, FakeHandler).await }
        }),
    )
}

fn stateless() -> Router {
    app(Arc::new(StreamTransport::new(true)))
}

fn post(body: &str) -> Request<Body> {
    request(
        "POST",
        body,
        "application/json, text/event-stream",
        "application/json",
    )
}

fn request(method: &str, body: &str, accept: &str, content_type: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::ACCEPT, accept)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body.to_owned()))
        .expect("a request")
}

fn get(accept: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/mcp")
        .header(header::ACCEPT, accept)
        .body(Body::empty())
        .expect("a request")
}

async fn send(router: &Router, request: Request<Body>) -> Response<Body> {
    router.clone().oneshot(request).await.expect("a response")
}

async fn body_bytes(response: Response<Body>) -> Vec<u8> {
    to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the response body")
        .to_vec()
}

async fn body_text(response: Response<Body>) -> String {
    String::from_utf8(body_bytes(response).await).expect("a utf-8 body")
}

#[derive(serde::Deserialize)]
struct Refusal {
    error: RefusalError,
}

#[derive(serde::Deserialize)]
struct RefusalError {
    code: i32,
    message: String,
}

async fn refusal(response: Response<Body>) -> Refusal {
    let text = String::from_utf8(body_bytes(response).await).expect("a utf-8 body");
    serde_json::from_str(&text).expect("a JSON-RPC error body")
}

#[rstest::rstest]
#[case::wildcard("*/*", true, true)]
#[case::both("application/json, text/event-stream", true, true)]
#[case::parameters("application/json;q=0.9, text/event-stream;q=0.8", true, true)]
#[case::json_only("application/json", true, false)]
#[case::sse_only("text/event-stream", false, true)]
#[case::application_wildcard("application/*", true, false)]
#[case::text_wildcard("text/*", false, true)]
fn reads_the_accept_header(#[case] value: &str, #[case] has_json: bool, #[case] has_sse: bool) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        HeaderValue::from_str(value).expect("a header"),
    );
    assert_eq!(super::accepted_media_types(&headers), (has_json, has_sse));
}

#[tokio::test]
async fn rejects_a_post_that_does_not_accept_event_stream() {
    let response = send(
        &stateless(),
        request(
            "POST",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "application/json",
            "application/json",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32600_i32);
    assert_eq!(
        body.error.message,
        "Not Acceptable: Client must accept both application/json and text/event-stream"
    );
}

#[tokio::test]
async fn rejects_a_post_with_another_content_type() {
    let response = send(
        &stateless(),
        request(
            "POST",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "application/json, text/event-stream",
            "text/plain",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32600_i32);
    assert_eq!(
        body.error.message,
        "Unsupported Media Type: Content-Type must be application/json"
    );
}

#[tokio::test]
async fn answers_an_unparsable_body_with_a_parse_error() {
    let response = send(&stateless(), post("{")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32700_i32);
    assert!(body.error.message.starts_with("Parse error: "));
}

#[tokio::test]
async fn answers_json_that_is_not_a_message_with_a_validation_error() {
    let response = send(&stateless(), post(r#"{"foo":1}"#)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32602_i32);
    assert!(body.error.message.starts_with("Validation error: "));
}

#[tokio::test]
async fn answers_an_oversized_body_with_payload_too_large() {
    let body = "x".repeat(4 * 1024 * 1024 + 1);
    let response = send(&stateless(), post(&body)).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body_text(response).await, "Request body too large");
}

#[tokio::test]
async fn answers_a_notification_with_accepted_and_no_body() {
    let response = send(
        &stateless(),
        post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert!(response.headers().get("mcp-session-id").is_none());
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn answers_a_client_response_with_accepted() {
    let response = send(
        &stateless(),
        post(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn ends_a_request_stream_after_its_response() {
    let response = send(
        &stateless(),
        post(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-cache, no-transform"))
    );
    let body = String::from_utf8(body_bytes(response).await).expect("a utf-8 body");
    assert_eq!(
        body,
        "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\r\n\r\n"
    );
}

#[test]
fn frames_an_sse_event_with_crlf_and_a_blank_line() {
    let message = ServerJsonRpcMessage::response(ServerResult::empty(()), RequestId::Number(2));
    assert_eq!(
        sse_event(&message),
        Bytes::from_static(
            b"event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\r\n\r\n"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn pings_an_open_stream_every_fifteen_seconds() {
    let response = send(&stateless(), get("text/event-stream")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let chunk = stream.next().await.expect("a ping").expect("a ping");
    let text = String::from_utf8(chunk.to_vec()).expect("a utf-8 ping");
    assert!(text.starts_with(": ping - "), "{text}");
    assert!(text.ends_with("\r\n\r\n"), "{text}");
}

#[test]
fn replays_the_mcp_jsonrpc_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mcp_jsonrpc_message");
    let entries: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus directory")
        .map(|entry| entry.expect("a corpus entry").path())
        .collect();
    assert!(!entries.is_empty(), "the corpus has no entries");
    replay(&entries);
}

fn replay(entries: &[PathBuf]) {
    for path in entries {
        let bytes = fs::read(path).expect("a corpus file");
        let _decoded = decode_message(&bytes);
    }
}
