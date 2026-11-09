#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, BodyDataStream, to_bytes};
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use axum::routing::any;
use futures_util::StreamExt as _;
use rmcp::ErrorData;
use rmcp::RoleServer;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, RequestId, ServerConfig, ServerRequest,
};
use rmcp::service::{MaybeSendFuture, PeerRequestOptions, RequestContext};
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

use crate::mcp::stream_transport::StreamTransport;

const INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"test-client","version":"1.0"}}}"#
);
const REJECTED_CLIENT: &str = "rejected-client";
const REJECTED_INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"rejected-client","version":"1.0"}}}"#
);
const TOOLS_LIST: &str = concat!(
    r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","#,
    r#""params":{"_meta":{"progressToken":"token-7"}}}"#
);
const SAMPLING_QUESTION: &str = concat!(
    r#"{"method":"sampling/createMessage","params":{"messages":"#,
    r#"[{"role":"user","content":{"type":"text","text":"what is the answer?"}}],"maxTokens":16}}"#
);

#[derive(Clone, Copy)]
struct FakeHandler;

impl ServerHandler for FakeHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::default()
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        let outcome = if request.client_info.name == REJECTED_CLIENT {
            Err(ErrorData::invalid_params("client rejected", None))
        } else {
            context.peer.set_peer_info(request.clone());
            self.negotiate_initialize(&request)
        };
        ready(outcome)
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
        let question: ServerRequest =
            serde_json::from_str(SAMPLING_QUESTION).expect("a sampling question");
        let handle = context
            .peer
            .send_cancellable_request(question, PeerRequestOptions::no_options())
            .await
            .expect("a sampling question sent");
        let _answer = handle.await_response().await;
        Ok(ListToolsResult::default())
    }
}

fn app() -> (Arc<StreamTransport>, Router) {
    let transport = Arc::new(StreamTransport::new(false));
    let router = Router::new().route(
        "/mcp",
        any({
            let shared = Arc::clone(&transport);
            move |request: Request<Body>| {
                let manager = Arc::clone(&shared);
                async move { manager.handle(request, FakeHandler).await }
            }
        }),
    );
    (transport, router)
}

fn post(body: &str, session: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(id) = session {
        builder = builder.header("mcp-session-id", id);
    }
    builder
        .body(Body::from(body.to_owned()))
        .expect("a request")
}

fn with_session(method: Method, accept: &str, session: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::ACCEPT, accept)
        .header("mcp-session-id", session)
        .body(Body::empty())
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

fn session_id(response: &Response<Body>) -> String {
    let value = response
        .headers()
        .get("mcp-session-id")
        .expect("a session id header");
    value.to_str().expect("an ASCII session id").to_owned()
}

fn is_hex_session_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    serde_json::from_str(&body_text(response).await).expect("a JSON-RPC error body")
}

#[derive(serde::Deserialize)]
struct Seen {
    id: Option<RequestId>,
    method: Option<String>,
    #[serde(default)]
    error: Option<SeenError>,
}

#[derive(serde::Deserialize)]
struct SeenError {
    code: i32,
    message: String,
}

fn data(event: &str) -> &str {
    event
        .strip_prefix("event: message\r\ndata: ")
        .and_then(|rest| rest.strip_suffix("\r\n\r\n"))
        .expect("an SSE message event")
}

async fn next_message(stream: &mut BodyDataStream) -> Seen {
    let chunk = stream.next().await.expect("a chunk").expect("a chunk");
    let event = String::from_utf8(chunk.to_vec()).expect("a utf-8 chunk");
    serde_json::from_str(data(&event)).expect("a JSON-RPC message")
}

async fn initialize(router: &Router) -> String {
    let response = send(router, post(INITIALIZE, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = session_id(&response);
    let _body = body_text(response).await;
    id
}

#[tokio::test]
async fn creates_a_session_from_initialize() {
    let (transport, router) = app();
    let response = send(&router, post(INITIALIZE, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    let id = session_id(&response);
    assert!(is_hex_session_id(&id), "{id}");
    let body = body_text(response).await;
    let reply: Seen = serde_json::from_str(data(&body)).expect("the initialize reply");
    assert_eq!(reply.id, Some(RequestId::Number(1)));
    assert_eq!(transport.live_session_ids(), vec![id.clone()]);
    assert!(transport.has_session(&id));
}

#[tokio::test]
async fn routes_later_requests_by_session_id() {
    let (transport, router) = app();
    let id = initialize(&router).await;
    let response = send(
        &router,
        post(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, Some(&id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_text(response).await,
        "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\r\n\r\n"
    );
    assert!(transport.has_session(&id));
}

#[tokio::test]
async fn rejects_a_post_without_a_session_id() {
    let (transport, router) = app();
    let response = send(&router, post(TOOLS_LIST, None)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let id = session_id(&response);
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32600_i32);
    assert_eq!(body.error.message, "Bad Request: Missing session ID");
    assert!(!transport.has_session(&id));
    assert!(transport.live_session_ids().is_empty());
}

#[tokio::test]
async fn rejects_an_unknown_session_id() {
    let (_, router) = app();
    let response = send(
        &router,
        post(
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            Some("ffffffffffffffffffffffffffffffff"),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("mcp-session-id").is_none());
    let body = refusal(response).await;
    assert_eq!(body.error.code, -32600_i32);
    assert_eq!(body.error.message, "Session not found");
}

#[tokio::test]
async fn answers_a_notification_with_accepted() {
    let (transport, router) = app();
    let id = initialize(&router).await;
    let response = send(
        &router,
        post(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            Some(&id),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers().get("mcp-session-id"),
        Some(&HeaderValue::from_str(&id).expect("a header"))
    );
    assert!(body_text(response).await.is_empty());
    assert!(transport.has_session(&id));
}

#[tokio::test]
async fn rejects_a_second_standalone_stream() {
    let (_, router) = app();
    let id = initialize(&router).await;
    let first = send(&router, with_session(Method::GET, "text/event-stream", &id)).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = send(&router, with_session(Method::GET, "text/event-stream", &id)).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let body = refusal(second).await;
    assert_eq!(
        body.error.message,
        "Conflict: Only one SSE stream is allowed per session"
    );
}

#[tokio::test]
async fn deletes_the_session() {
    let (transport, router) = app();
    let id = initialize(&router).await;
    let response = send(&router, with_session(Method::DELETE, "*/*", &id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert!(body_text(response).await.is_empty());
    assert!(!transport.has_session(&id));
    let later = send(
        &router,
        post(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, Some(&id)),
    )
    .await;
    assert_eq!(later.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn terminate_ends_a_session_and_its_streams() {
    let (transport, router) = app();
    let id = initialize(&router).await;
    let stream = send(&router, with_session(Method::GET, "text/event-stream", &id)).await;
    assert_eq!(stream.status(), StatusCode::OK);
    assert!(transport.terminate(&id));
    assert!(!transport.has_session(&id));
    assert!(!transport.terminate(&id));
    let body = timeout(
        Duration::from_secs(5),
        to_bytes(stream.into_body(), 1024 * 1024),
    )
    .await
    .expect("the open stream to end")
    .expect("an empty body");
    assert!(body.is_empty());
}

#[tokio::test]
async fn lists_live_sessions_in_creation_order() {
    let (transport, router) = app();
    let first = initialize(&router).await;
    let second = initialize(&router).await;
    assert_eq!(
        transport.live_session_ids(),
        vec![first.clone(), second.clone()]
    );
    assert!(transport.terminate(&first));
    assert_eq!(transport.live_session_ids(), vec![second]);
    assert!(!transport.has_session(&first));
}

#[derive(serde::Serialize)]
struct ClientAnswer {
    jsonrpc: &'static str,
    id: RequestId,
    result: AnswerResult,
}

#[derive(serde::Serialize)]
struct AnswerResult {
    model: &'static str,
    role: &'static str,
    content: AnswerContent,
}

#[derive(serde::Serialize)]
struct AnswerContent {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'static str,
}

#[tokio::test]
async fn streams_progress_and_a_sampling_answer_on_the_requests_stream() {
    let (transport, router) = app();
    let id = initialize(&router).await;
    let response = send(&router, post(TOOLS_LIST, Some(&id))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("mcp-session-id"),
        Some(&HeaderValue::from_str(&id).expect("a header"))
    );
    let mut stream = response.into_body().into_data_stream();
    let progress = next_message(&mut stream).await;
    assert_eq!(progress.method.as_deref(), Some("notifications/progress"));
    let question = next_message(&mut stream).await;
    assert_eq!(question.method.as_deref(), Some("sampling/createMessage"));
    let question_id = question.id.expect("a sampling request id");
    let answer = ClientAnswer {
        jsonrpc: "2.0",
        id: question_id,
        result: AnswerResult {
            model: "test-model",
            role: "assistant",
            content: AnswerContent {
                kind: "text",
                text: "the answer",
            },
        },
    };
    let body = serde_json::to_string(&answer).expect("the client answer");
    let accepted = send(&router, post(&body, Some(&id))).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let reply = next_message(&mut stream).await;
    assert_eq!(reply.id, Some(RequestId::Number(7)));
    assert!(stream.next().await.is_none(), "the request stream ended");
    assert!(transport.has_session(&id));
}

#[tokio::test]
async fn ends_a_cancelled_requests_stream_with_the_sdk_error() {
    let (_, router) = app();
    let id = initialize(&router).await;
    let response = send(&router, post(TOOLS_LIST, Some(&id))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let _progress = next_message(&mut stream).await;
    let _question = next_message(&mut stream).await;
    let cancel = concat!(
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","#,
        r#""params":{"requestId":7,"reason":"user"}}"#
    );
    let accepted = send(&router, post(cancel, Some(&id))).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let reply = next_message(&mut stream).await;
    assert_eq!(reply.id, Some(RequestId::Number(7)));
    let error = reply.error.expect("the cancellation error");
    assert_eq!(error.code, -32800_i32);
    assert_eq!(error.message, "Request cancelled");
    assert!(stream.next().await.is_none(), "the request stream ended");
}

#[tokio::test]
async fn discards_a_session_whose_initialize_fails() {
    let (transport, router) = app();
    let response = send(&router, post(REJECTED_INITIALIZE, None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = session_id(&response);
    let _body = body_text(response).await;
    wait_for_no_session(&transport, &id).await;
    assert!(transport.live_session_ids().is_empty());
}

#[tokio::test]
async fn keeps_a_session_after_an_undecodable_initialize() {
    let (transport, router) = app();
    let response = send(
        &router,
        post(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            None,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = session_id(&response);
    let body = body_text(response).await;
    assert!(body.contains(r#""error""#), "{body}");
    assert!(transport.has_session(&id));
}

async fn wait_for_no_session(transport: &StreamTransport, id: &str) {
    timeout(Duration::from_secs(5), async {
        while transport.has_session(id) {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the failed session to be discarded");
}
