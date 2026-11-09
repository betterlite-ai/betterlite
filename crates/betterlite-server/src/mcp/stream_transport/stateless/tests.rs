#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use axum::routing::any;
use rmcp::ErrorData;
use rmcp::RoleServer;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, RequestId, ServerConfig,
};
use rmcp::service::RequestContext;
use tokio::time::sleep;
use tower::ServiceExt as _;

use crate::mcp::stream_transport::StreamTransport;

const TOOLS_LIST: &str = concat!(
    r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","#,
    r#""params":{"_meta":{"progressToken":"token-7"}}}"#
);

#[derive(Clone, Copy)]
struct FakeHandler {
    slow: bool,
}

impl ServerHandler for FakeHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::default()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if self.slow {
            sleep(Duration::from_secs(6)).await;
        }
        if let Some(token) = context.meta.get_progress_token() {
            let _sent = context
                .peer
                .notify_progress(ProgressNotificationParam::new(token, 1.0))
                .await;
        }
        Ok(ListToolsResult::default())
    }
}

fn app<H>(handler: H) -> (Arc<StreamTransport>, Router)
where
    H: ServerHandler + Clone,
{
    let transport = Arc::new(StreamTransport::new(true));
    let router = Router::new().route(
        "/mcp",
        any({
            let shared = Arc::clone(&transport);
            move |request: Request<Body>| {
                let manager = Arc::clone(&shared);
                let per_request = handler.clone();
                async move { manager.handle(request, per_request).await }
            }
        }),
    );
    (transport, router)
}

fn request(method: Method, accept: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::ACCEPT, accept)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("a request")
}

fn post(body: &str) -> Request<Body> {
    request(
        Method::POST,
        "application/json, text/event-stream",
        Body::from(body.to_owned()),
    )
}

fn get(accept: &str) -> Request<Body> {
    request(Method::GET, accept, Body::empty())
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

#[derive(serde::Deserialize)]
struct Seen {
    id: Option<RequestId>,
    method: Option<String>,
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

#[tokio::test]
async fn serves_a_request_without_initialize() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(&router, post(TOOLS_LIST)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    let events = events(&body);
    let progress: Seen = serde_json::from_str(data(events[0])).expect("the progress notification");
    assert_eq!(progress.method.as_deref(), Some("notifications/progress"));
    let reply: Seen = serde_json::from_str(data(events[1])).expect("the tools/list reply");
    assert_eq!(reply.id, Some(RequestId::Number(7)));
    assert!(reply.method.is_none());
}

#[tokio::test(start_paused = true)]
async fn keeps_the_service_alive_while_a_slow_handler_runs() {
    let (_, router) = app(FakeHandler { slow: true });
    let response = send(&router, post(TOOLS_LIST)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(body.contains("\"id\":7"), "{body}");
    assert!(body.contains("\"result\""), "{body}");
}

#[tokio::test]
async fn keeps_no_session_after_a_request() {
    let (transport, router) = app(FakeHandler { slow: false });
    let response = send(&router, post(TOOLS_LIST)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _body = body_text(response).await;
    assert!(transport.live_session_ids().is_empty());
    assert!(!transport.has_session("00000000000000000000000000000000"));
}

#[tokio::test]
async fn answers_a_notification_without_initialize() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(
        &router,
        post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
}

#[tokio::test]
async fn rejects_a_get_that_does_not_accept_event_stream() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(&router, get("application/json")).await;
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn opens_a_standalone_stream_for_get() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(&router, get("text/event-stream")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );
    assert!(response.headers().get("mcp-session-id").is_none());
}

#[tokio::test]
async fn answers_a_delete_as_unsupported() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(&router, request(Method::DELETE, "*/*", Body::empty())).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let body = body_text(response).await;
    assert!(
        body.contains("Method Not Allowed: Session termination not supported"),
        "{body}"
    );
}

#[tokio::test]
async fn answers_an_unsupported_method_with_allow() {
    let (_, router) = app(FakeHandler { slow: false });
    let response = send(&router, request(Method::PUT, "*/*", Body::empty())).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(header::ALLOW),
        Some(&HeaderValue::from_static("GET, POST, DELETE"))
    );
}
