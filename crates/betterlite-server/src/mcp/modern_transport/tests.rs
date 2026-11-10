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
    CallToolRequestParams, CallToolResponse, CallToolResult, ElicitRequest, ElicitRequestParams,
    ElicitationSchema, ErrorCode, InputRequiredResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, RequestId, ServerConfig, ServerRequest,
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
const GET_PROMPT: &str = concat!(
    r#"{"jsonrpc":"2.0","id":11,"method":"prompts/get","params":{"#,
    r#""name":"test-prompt","_meta":{"#,
    r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
    r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
);
const READ_RESOURCE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":12,"method":"resources/read","params":{"#,
    r#""uri":"test://resource","_meta":{"#,
    r#""io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
    r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
);
const BASE64_TOOL_NAME: &str = "=?base64?dGVzdC10b29s?=";
const CORRUPT_NAME_CALL: &str = concat!(
    r#"{"jsonrpc":"2.0","id":34,"method":"tools/call","params":{"name":"NAME","#,
    r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
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
    AnswerAfterBackChannel,
    NotifyThenBackChannel,
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
        if matches!(
            self.behaviour,
            Behaviour::NotifyThenAnswer | Behaviour::NotifyThenBackChannel
        ) && let Some(token) = context.meta.get_progress_token()
        {
            let _sent = context
                .peer
                .notify_progress(ProgressNotificationParam::new(token, 1.0))
                .await;
        }
        if matches!(
            self.behaviour,
            Behaviour::AnswerAfterBackChannel | Behaviour::NotifyThenBackChannel
        ) {
            let requested_schema = ElicitationSchema::builder()
                .required_email("email")
                .build()
                .expect("an elicitation schema");
            let params = ElicitRequestParams::FormElicitationParams {
                meta: None,
                message: "the fake handler asks".to_owned(),
                requested_schema,
            };
            let request = ServerRequest::ElicitRequest(ElicitRequest::new(params));
            let _refused = context.peer.send_request(request).await;
        }
        if self.behaviour == Behaviour::Slow {
            sleep(Duration::from_secs(20)).await;
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

fn envelope_post(accept: Option<&str>, content_type: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/list");
    let with_type = match content_type {
        Some(value) => builder.header(header::CONTENT_TYPE, value),
        None => builder,
    };
    let with_accept = match accept {
        Some(value) => with_type.header(header::ACCEPT, value),
        None => with_type,
    };
    with_accept
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request")
}

fn version_headers(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(
            "mcp-protocol-version",
            HeaderValue::from_str(value).expect("a header"),
        );
    }
    headers
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

async fn refusal(response: Response<Body>) -> Refusal {
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
    let headers = version.map_or_else(HeaderMap::new, |value| version_headers(&[value]));
    assert_eq!(takes_modern_path(&headers), modern);
}

#[test]
fn routes_by_the_first_duplicate_version_header() {
    assert!(takes_modern_path(&version_headers(&[
        "2026-07-28",
        "2025-06-18"
    ])));
    assert!(!takes_modern_path(&version_headers(&[
        "2025-06-18",
        "2026-07-28"
    ])));
}

#[test]
fn routes_a_version_header_that_is_not_utf8() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "mcp-protocol-version",
        HeaderValue::from_bytes(b"2026-07-28\x80").expect("a header"),
    );
    assert!(takes_modern_path(&headers));
}

#[tokio::test]
async fn answers_a_request_with_json_and_no_session_id() {
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/list")
        .header("mcp-session-id", "0123456789abcdef0123456789abcdef")
        .body(Body::from(LIST_TOOLS.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert!(response.headers().get("mcp-session-id").is_none());
    let body = body_text(response).await;
    let reply: Message = serde_json::from_str(&body).expect("a JSON-RPC reply");
    assert_eq!(reply.id, Some(RequestId::Number(7)));
    assert!(reply.method.is_none());
    assert!(body.contains("\"result\""), "{body}");
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
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28"],"requested":""}}}"#
    );
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
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Body must be a single JSON-RPC request or notification object"}}"#
    );
}

#[tokio::test]
async fn answers_a_parse_error() {
    let router = app(answers());
    let response = send(&router, modern_request(r#"{"jsonrpc":"2.0"#, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#
    );
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
async fn answers_a_mismatched_jsonrpc_version_as_invalid() {
    let body = r#"{"jsonrpc":"1.0","id":3,"method":"tools/list","params":{}}"#;
    let response = send(&app(answers()), modern_request(body, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Body must be a single JSON-RPC request or notification object"}}"#
    );
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

#[rstest]
#[case::missing(None, StatusCode::BAD_REQUEST)]
#[case::text(Some("text/plain"), StatusCode::BAD_REQUEST)]
#[case::json(Some("application/json"), StatusCode::OK)]
#[case::json_with_charset(Some("Application/JSON; charset=utf-8"), StatusCode::OK)]
#[tokio::test]
async fn checks_the_content_type(#[case] content_type: Option<&str>, #[case] expected: StatusCode) {
    let request = envelope_post(Some("application/json, text/event-stream"), content_type);
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), expected);
    if expected == StatusCode::BAD_REQUEST {
        assert!(response.headers().get(header::CONTENT_TYPE).is_none());
        assert_eq!(body_text(response).await, "Invalid Content-Type header");
    }
}

#[rstest]
#[case::both(Some("application/json, text/event-stream"), StatusCode::OK)]
#[case::wildcard(Some("*/*"), StatusCode::OK)]
#[case::wildcard_types(Some("application/*, text/*"), StatusCode::OK)]
#[case::json_only(Some("application/json"), StatusCode::NOT_ACCEPTABLE)]
#[case::event_stream_only(Some("text/event-stream"), StatusCode::NOT_ACCEPTABLE)]
#[case::missing(None, StatusCode::NOT_ACCEPTABLE)]
#[tokio::test]
async fn checks_the_accept_media_ranges(
    #[case] accept: Option<&str>,
    #[case] expected: StatusCode,
) {
    let response = send(
        &app(answers()),
        envelope_post(accept, Some("application/json")),
    )
    .await;
    assert_eq!(response.status(), expected);
}

#[tokio::test]
async fn refuses_an_oversized_body() {
    let router = app(answers());
    let oversized = "x".repeat(MAX_REQUEST_BYTES.saturating_add(1));
    let response = send(&router, modern_request(&oversized, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn accepts_a_body_just_under_the_limit() {
    let mut body = LIST_TOOLS.to_owned();
    let padding = MAX_REQUEST_BYTES
        .saturating_sub(body.len())
        .saturating_sub(1);
    body.push_str(&" ".repeat(padding));
    assert!(body.len() < MAX_REQUEST_BYTES);
    let response = send(&app(answers()), modern_request(&body, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn refuses_a_request_without_the_envelope_pair() {
    let router = app(answers());
    let response = send(&router, modern_request(NO_META, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32602_i32);
    assert_eq!(refusal.id, Some(RequestId::Number(7)));
    assert_eq!(
        refusal.error.message,
        "params._meta is missing the required envelope key(s): io.modelcontextprotocol/protocolVersion, io.modelcontextprotocol/clientCapabilities"
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

#[rstest]
#[case::call_tool(CALL_TOOL, "tools/call", "other-tool", "'name'")]
#[case::get_prompt(GET_PROMPT, "prompts/get", "other-prompt", "'name'")]
#[case::read_resource(READ_RESOURCE, "resources/read", "other-uri", "'uri'")]
#[tokio::test]
async fn refuses_a_name_header_that_disagrees_with_the_body(
    #[case] body: &'static str,
    #[case] method: &'static str,
    #[case] name: &'static str,
    #[case] parameter: &'static str,
) {
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", method)
        .header("mcp-name", name)
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        format!("mcp-name header does not match the request body's {parameter} parameter")
    );
}

#[tokio::test]
async fn accepts_a_base64_name_header_that_matches() {
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", BASE64_TOOL_NAME)
        .body(Body::from(CALL_TOOL.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[rstest]
#[case::a_bad_payload("=?base64?not base64?=", "=?base64?not base64?=")]
#[case::a_non_canonical_payload("a", "=?base64?YR==?=")]
#[tokio::test]
async fn refuses_a_malformed_base64_name_header(
    #[case] body_name: &str,
    #[case] name_header: &str,
) {
    let body = CORRUPT_NAME_CALL.replace("NAME", body_name);
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", name_header)
        .body(Body::from(body))
        .expect("a request");
    let response = send(&app(answers()), request).await;
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
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28"],"requested":"2027-01-01"}}}"#
    );
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
async fn refuses_an_initialize_on_the_modern_path() {
    let body = concat!(
        r#"{"jsonrpc":"2.0","id":21,"method":"initialize","params":{"#,
        r#""protocolVersion":"2026-07-28","capabilities":{},"#,
        r#""clientInfo":{"name":"test-client","version":"1.0"},"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
        r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
    );
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "initialize")
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":21,"error":{"code":-32601,"message":"Method not found","data":"initialize"}}"#
    );
}

#[tokio::test]
async fn refuses_a_tool_call_without_its_name() {
    let body = concat!(
        r#"{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"arguments":{},"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
        r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
    );
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", "test-tool")
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":31,"error":{"code":-32602,"message":"Invalid request parameters","data":""}}"#
    );
}

#[tokio::test]
async fn treats_a_null_tool_name_as_absent() {
    let body = concat!(
        r#"{"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":null,"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
        r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
    );
    let request = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call")
        .header("mcp-name", "test-tool")
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32602_i32);
    assert_eq!(refusal.error.message, "Invalid request parameters");
}

#[rstest]
#[case::with_the_name_header(Some("test-tool"))]
#[case::without_the_name_header(None)]
#[tokio::test]
async fn refuses_a_tool_call_with_a_numeric_name(#[case] name_header: Option<&str>) {
    let body = concat!(
        r#"{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":5,"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","#,
        r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
    );
    let builder = accepted_post()
        .header("mcp-protocol-version", PROTOCOL_VERSION)
        .header("mcp-method", "tools/call");
    let with_name = match name_header {
        Some(value) => builder.header("mcp-name", value),
        None => builder,
    };
    let request = with_name
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let response = send(&app(answers()), request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, -32020_i32);
    assert_eq!(
        refusal.error.message,
        "mcp-name header does not match the request body's 'name' parameter"
    );
}

#[tokio::test]
async fn refuses_a_meta_that_is_not_an_object() {
    let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":5}}"#;
    let response = send(&app(answers()), modern_request(body, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32602,"message":"params._meta must be an object carrying the required 'io.modelcontextprotocol/protocolVersion' and 'io.modelcontextprotocol/clientCapabilities' envelope keys"}}"#
    );
}

#[tokio::test]
async fn refuses_a_numeric_protocol_version() {
    let body = concat!(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":{"#,
        r#""io.modelcontextprotocol/protocolVersion":20260728,"#,
        r#""io.modelcontextprotocol/clientCapabilities":{}}}}"#
    );
    let response = send(&app(answers()), modern_request(body, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(response).await,
        r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32020,"message":"mcp-protocol-version header does not match the request envelope's protocol version"}}"#
    );
}

#[rstest]
#[case::method_not_found(ErrorCode::METHOD_NOT_FOUND, StatusCode::NOT_FOUND, -32601_i32)]
#[case::missing_capability(
    ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY,
    StatusCode::BAD_REQUEST,
    -32021_i32
)]
#[case::invalid_params(ErrorCode::INVALID_PARAMS, StatusCode::BAD_REQUEST, -32602_i32)]
#[case::internal(ErrorCode::INTERNAL_ERROR, StatusCode::OK, -32603_i32)]
#[tokio::test]
async fn maps_handler_errors_through_the_status_table(
    #[case] failure: ErrorCode,
    #[case] status: StatusCode,
    #[case] code: i32,
) {
    let router = app(FakeHandler {
        behaviour: Behaviour::Answer,
        failure: Some(failure),
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), status);
    let refusal: Refusal = refusal(response).await;
    assert_eq!(refusal.error.code, code);
}

#[tokio::test]
async fn keeps_a_refused_server_request_off_the_wire() {
    let router = app(FakeHandler {
        behaviour: Behaviour::AnswerAfterBackChannel,
        failure: None,
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(!body.contains("elicitation/create"), "{body}");
    assert!(body.contains("\"id\":7"), "{body}");
}

#[tokio::test]
async fn keeps_a_refused_server_request_out_of_a_stream() {
    let router = app(FakeHandler {
        behaviour: Behaviour::NotifyThenBackChannel,
        failure: None,
    });
    let response = send(&router, modern_request(LIST_TOOLS, "tools/list")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    let body = body_text(response).await;
    assert!(!body.contains("elicitation/create"), "{body}");
    assert_eq!(events(&body).len(), 2, "{body}");
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
