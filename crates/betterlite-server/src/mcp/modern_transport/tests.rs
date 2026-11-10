#![cfg(test)]

use std::future::{Future, ready};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::request::Builder;
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode, header};
use axum::routing::any;
use rmcp::ErrorData as McpError;
use rmcp::RoleServer;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, InputRequiredResult,
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, RequestId, ServerConfig,
};
use rmcp::service::RequestContext;
use rstest::rstest;
use serde::Deserialize;
use tokio::time::sleep;
use tower::ServiceExt as _;

use super::{MAX_REQUEST_BYTES, ModernTransport, takes_modern_path};

const PROTOCOL_VERSION: &str = "2026-07-28";
const LIST_TOOLS: &str = concat!(
    r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":{"#,
    r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
    r#""io.modelcontextprotocol/clientCapabilities":{},"progressToken":"token-7"}}}"#
);
const CALL_TOOL: &str = concat!(
    r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"#,
    r#""name":"test-tool","arguments":{},"_meta":{"#,
    r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
    r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
);
const NOTIFICATION: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const NO_META: &str = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}"#;
const NO_CAPABILITIES: &str = concat!(
    r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":{"#,
    r#""io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    Answer,
    NotifyThenAnswer,
    Slow,
    InputRequired,
}

#[derive(Clone, Copy)]
struct FakeHandler {
    behaviour: Behaviour,
    failure: Option<ErrorCode>,
}

impl ServerHandler for FakeHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::default()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if let Some(code) = self.failure {
            return Err(McpError::new(code, "the fake handler failed", None));
        }
        match self.behaviour {
            Behaviour::NotifyThenAnswer => {
                if let Some(token) = context.meta.get_progress_token() {
                    let _sent = context
                        .peer
                        .notify_progress(ProgressNotificationParam::new(token, 1.0))
                        .await;
                }
            }
            Behaviour::Slow => sleep(Duration::from_secs(20)).await,
            Behaviour::Answer | Behaviour::InputRequired => {}
        }
        Ok(ListToolsResult::default())
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + Send + '_ {
        let response = if self.behaviour == Behaviour::InputRequired {
            let result = InputRequiredResult::new(None, Some("state-1".to_owned()));
            CallToolResponse::InputRequired(result)
        } else {
            CallToolResponse::Complete(CallToolResult::success(Vec::new()))
        };
        ready(Ok(response))
    }
}

fn app(handler: FakeHandler) -> Router {
    Router::new().route(
        "/mcp",
        any(move |request: Request<Body>| async move {
            ModernTransport.handle(request, handler).await
        }),
    )
}

fn post() -> Builder {
    Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
}

fn accepted_post() -> Builder {
    post().header(header::ACCEPT, "application/json, text/event-stream")
}

fn modern_request(body: &str, method: &str) -> Request<Body> {
    accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", method)
        .body(Body::from(body.to_owned()))
        .expect("a request")
}

fn call_tool_request() -> Request<Body> {
    accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", "test-tool")
        .body(Body::from(CALL_TOOL.to_owned()))
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

fn events(body: &str) -> Vec<&str> {
    body.split("\r\n\r\n")
        .filter(|event| !event.is_empty())
        .collect()
}

fn data(event: &str) -> &str {
    event
        .strip_prefix("event: message\r\ndata: ")
        .expect("an SSE message event")
}

#[derive(Deserialize)]
struct Message {
    id: Option<RequestId>,
    method: Option<String>,
}

#[derive(Deserialize)]
struct Refusal {
    id: Option<RequestId>,
    error: RefusalError,
}

#[derive(Deserialize)]
struct RefusalError {
    code: i32,
    message: String,
}

#[derive(Deserialize)]
struct VersionRefusal {
    error: VersionRefusalError,
}

#[derive(Deserialize)]
struct VersionRefusalError {
    code: i32,
    message: String,
    data: VersionData,
}

#[derive(Deserialize)]
struct VersionData {
    supported: Vec<String>,
    requested: String,
}

async fn refusal(response: Response<Body>) -> Refusal {
    let text = String::from_utf8(body_bytes(response).await).expect("a utf-8 body");
    serde_json::from_str(&text).expect("a JSON-RPC error body")
}

async fn version_refusal(response: Response<Body>) -> VersionRefusal {
    let text = String::from_utf8(body_bytes(response).await).expect("a utf-8 body");
    serde_json::from_str(&text).expect("a JSON-RPC error body")
}

fn answers() -> FakeHandler {
    FakeHandler {
        behaviour: Behaviour::Answer,
        failure: None,
    }
}

#[rstest]
#[case::missing(None, false)]
#[case::handshake_2024(Some("2024-11-05"), false)]
#[case::handshake_2025_03(Some("2025-03-26"), false)]
#[case::handshake_2025_06(Some("2025-06-18"), false)]
#[case::handshake_2025_11(Some("2025-11-25"), false)]
#[case::modern(Some("2026-07-28"), true)]
#[case::unknown(Some("2027-01-01"), true)]
#[case::empty(Some(""), true)]
fn routes_the_protocol_version_header(#[case] version: Option<&str>, #[case] modern: bool) {
    let mut headers = HeaderMap::new();
    if let Some(text) = version {
        headers.insert(
            "mcp-protocol-version",
            HeaderValue::from_str(text).expect("a header"),
        );
    }
    assert_eq!(takes_modern_path(&headers), modern);
}

#[test]
fn routes_by_the_first_duplicate_version_header() {
    let mut headers = HeaderMap::new();
    headers.append(
        "mcp-protocol-version",
        HeaderValue::from_static("2026-07-28"),
    );
    headers.append(
        "mcp-protocol-version",
        HeaderValue::from_static("2025-06-18"),
    );
    assert!(takes_modern_path(&headers));

    let mut reversed = HeaderMap::new();
    reversed.append(
        "mcp-protocol-version",
        HeaderValue::from_static("2025-06-18"),
    );
    reversed.append(
        "mcp-protocol-version",
        HeaderValue::from_static("2026-07-28"),
    );
    assert!(!takes_modern_path(&reversed));
}

#[tokio::test]
async fn answers_a_request_with_a_json_body() {
    let router = app(answers());
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    let body = body_text(response).await;
    let reply: Message = serde_json::from_str(&body).expect("a JSON-RPC reply");
    assert_eq!(reply.id, Some(RequestId::Number(7)));
    assert!(reply.method.is_none());
    assert!(body.contains("\"result\""), "{body}");
}

#[tokio::test]
async fn carries_no_session_id_on_this_path() {
    let router = app(answers());
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/list")
        .header("mcp-session-id", "0123456789abcdef0123456789abcdef")
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("mcp-session-id").is_none());
}

#[tokio::test]
async fn streams_a_notification_before_the_reply() {
    let router = app(FakeHandler {
        behaviour: Behaviour::NotifyThenAnswer,
        failure: None,
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-cache, no-transform"))
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
    let body = body_text(response).await;
    assert!(!body.starts_with(": ping"), "{body}");
    let events = events(&body);
    assert_eq!(events.len(), 2, "{body}");
    let progress: Message =
        serde_json::from_str(data(events[0])).expect("the progress notification");
    assert_eq!(progress.method.as_deref(), Some("notifications/progress"));
    let reply: Message = serde_json::from_str(data(events[1])).expect("the tools/list reply");
    assert_eq!(reply.id, Some(RequestId::Number(7)));
}

#[tokio::test(start_paused = true)]
async fn pings_a_handler_past_the_deferral_window() {
    let router = app(FakeHandler {
        behaviour: Behaviour::Slow,
        failure: None,
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    let body = body_text(response).await;
    assert!(body.starts_with(": ping\r\n\r\n"), "{body}");
    assert!(body.contains("\"id\":7"), "{body}");
}

#[tokio::test]
async fn acknowledges_a_notification_with_202() {
    let router = app(answers());
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .body(Body::from(NOTIFICATION.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(response.headers().get(header::CONTENT_TYPE).is_none());
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn refuses_a_notification_at_an_unserved_version() {
    let router = app(answers());
    let request = accepted_post()
        .body(Body::from(NOTIFICATION.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: VersionRefusal = version_refusal(response).await;
    assert_eq!(refusal.error.code, -32022_i32);
    assert_eq!(refusal.error.message, "Unsupported protocol version");
    assert_eq!(refusal.error.data.supported, ["2026-07-28"]);
    assert_eq!(refusal.error.data.requested, "");
}

#[tokio::test]
async fn refuses_a_posted_response() {
    let router = app(answers());
    let response = send(
        &router,
        modern_request(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#, "tools/list"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32600_i32);
    assert_eq!(
        refusal.error.message,
        "Body must be a single JSON-RPC request or notification object"
    );
    assert_eq!(refusal.id, None);
}

#[tokio::test]
async fn answers_a_parse_error() {
    let router = app(answers());
    let response = send(&router, modern_request(r#"{"jsonrpc":"2.0"#, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32700_i32);
    assert_eq!(refusal.error.message, "Parse error");
}

#[tokio::test]
async fn answers_an_invalid_jsonrpc_body() {
    let router = app(answers());
    let response = send(
        &router,
        modern_request(r#"{"jsonrpc":"2.0","id":3}"#, "tools/list"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32600_i32);
}

#[tokio::test]
async fn answers_a_non_post_with_allow_post() {
    let router = app(answers());
    let request = Request::builder()
        .method("GET")
        .uri("/mcp")
        .header(header::ACCEPT, "*/*")
        .body(Body::empty())
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(header::ALLOW),
        Some(&HeaderValue::from_static("POST"))
    );
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn refuses_an_accept_without_event_stream() {
    let router = app(answers());
    let request = post()
        .header(header::ACCEPT, "application/json")
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/list")
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn refuses_an_oversized_body() {
    let router = app(answers());
    let oversized = "x".repeat(MAX_REQUEST_BYTES + 1);
    let response = send(&router, modern_request(&oversized, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn refuses_a_request_without_the_envelope_pair() {
    let router = app(answers());
    let response = send(&router, modern_request(NO_META, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32602_i32);
    assert_eq!(refusal.id, Some(RequestId::Number(7)));
    assert!(
        refusal
            .error
            .message
            .contains("io.modelcontextprotocol/protocolVersion"),
        "{}",
        refusal.error.message
    );
    assert!(
        refusal
            .error
            .message
            .contains("io.modelcontextprotocol/clientCapabilities"),
        "{}",
        refusal.error.message
    );
}

#[tokio::test]
async fn refuses_a_request_missing_the_capabilities_key() {
    let router = app(answers());
    let response = send(&router, modern_request(NO_CAPABILITIES, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32602_i32);
    assert_eq!(
        refusal.error.message,
        "params._meta is missing the required envelope key(s): io.modelcontextprotocol/clientCapabilities"
    );
}

#[tokio::test]
async fn refuses_a_version_header_that_disagrees_with_the_body() {
    let router = app(answers());
    let request = accepted_post()
        .header("mcp-protocol-version", "2026-07-29")
        .header("mcp-method", "tools/list")
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        "mcp-protocol-version header does not match the request envelope's protocol version"
    );
}

#[tokio::test]
async fn refuses_a_method_header_that_disagrees_with_the_body() {
    let router = app(answers());
    let response = send(&router, modern_request(LIST_TOOLS, "tools/call")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        "mcp-method header does not match the request body's method"
    );
}

#[tokio::test]
async fn refuses_a_name_header_that_disagrees_with_the_body() {
    let router = app(answers());
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", "other-tool")
        .body(Body::from(CALL_TOOL.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        "mcp-name header does not match the request body's 'name' parameter"
    );
}

#[tokio::test]
async fn refuses_a_version_outside_the_modern_set() {
    let router = app(answers());
    let body = LIST_TOOLS.replace(PROTOCOL_VERSION, "2027-01-01");
    let request = accepted_post()
        .header("mcp-protocol-version", "2027-01-01")
        .header("mcp-method", "tools/list")
        .body(Body::from(body))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: VersionRefusal = version_refusal(response).await;
    assert_eq!(refusal.error.code, -32022_i32);
    assert_eq!(refusal.error.message, "Unsupported protocol version");
    assert_eq!(refusal.error.data.supported, ["2026-07-28"]);
    assert_eq!(refusal.error.data.requested, "2027-01-01");
}

#[tokio::test]
async fn refuses_a_duplicated_routing_header() {
    let router = app(answers());
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/list")
        .header("mcp-method", "tools/call")
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request");
    let response = send(&router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        "mcp-method header appears more than once"
    );
}

#[tokio::test]
async fn maps_a_method_not_found_error_to_404() {
    let router = app(FakeHandler {
        behaviour: Behaviour::Answer,
        failure: Some(ErrorCode::METHOD_NOT_FOUND),
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32601_i32);
}

#[tokio::test]
async fn maps_an_invalid_params_error_to_400() {
    let router = app(FakeHandler {
        behaviour: Behaviour::Answer,
        failure: Some(ErrorCode::INVALID_PARAMS),
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32602_i32);
}

#[tokio::test]
async fn maps_an_internal_error_to_200() {
    let router = app(FakeHandler {
        behaviour: Behaviour::Answer,
        failure: Some(ErrorCode::INTERNAL_ERROR),
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32603_i32);
}

#[tokio::test]
async fn passes_an_input_required_result_through() {
    let router = app(FakeHandler {
        behaviour: Behaviour::InputRequired,
        failure: None,
    });
    let response = send(&router, call_tool_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    let body = body_text(response).await;
    assert!(body.contains("\"resultType\":\"input_required\""), "{body}");
    assert!(body.contains("\"requestState\":\"state-1\""), "{body}");
}
