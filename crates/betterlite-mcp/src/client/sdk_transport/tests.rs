#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request as IncomingRequest, State};
use axum::response::Response;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use bytes::Bytes;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LOCATION};
use http::{Method, StatusCode};
use rmcp::RoleClient;
use rmcp::model::{
    ClientNotification, ClientRequest, CustomNotification, CustomRequest, InitializedNotification,
    JsonRpcMessage, RequestId,
};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rstest::rstest;
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::client::cleanup::http_client::McpHttpClient;
use crate::client::http_transport::capture::{BodyChunks, CapturedBody, ErrorCapture, Exchange};
use crate::client::http_transport::send::{HttpBase, McpHttpTransport};
use crate::client::sessions::factory::{
    AuthSource, HttpClientConfig, TlsSettings, TlsVerification,
};
use crate::client::sessions::negotiate::{ClientSession, InitializeOutcome, InitializeRequest};
use crate::client::sessions::runtime::{McpRuntime as _, SessionOptions};
use crate::client::sessions::stdio::StdioStreams;
use crate::client::sessions::stdio_runtime::{StdioRuntime, StdioSessionFactory};
use crate::client::sessions::stream::{MessageHandler, StreamFailureKind};
use crate::client::sessions::transport::{StdioServerParameters, TransportContext};

use super::context::{SdkStreams, SdkTransport};
use super::error::SdkError;
use super::legacy_sse::LegacySse;
use super::origin::next_request_within_origin;
use super::streamable::StreamableHttp;

const BODY_LIMIT: usize = 0x0010_0000;
const SESSION_HEADER: HeaderName = HeaderName::from_static("mcp-session-id");
const VERSION_HEADER: HeaderName = HeaderName::from_static("mcp-protocol-version");
const LAST_EVENT_HEADER: HeaderName = HeaderName::from_static("last-event-id");

fn defaults() -> HttpClientConfig {
    HttpClientConfig {
        headers: BTreeMap::new(),
        timeout: std::time::Duration::from_secs(5),
        tls: TlsSettings {
            verification: TlsVerification::Enabled,
            ca_file: None,
        },
        auth: AuthSource::None,
        credential: None,
        follow_redirects: true,
        redirect_guard: None,
        capture_error_responses: false,
    }
}

fn base() -> HttpBase {
    let trust = ProxyTrust::default();
    let environment = BTreeMap::new();
    HttpBase {
        proxy: Arc::new(ProxyPolicy::resolve(&trust, &environment).unwrap()),
        tls: TlsPolicy {
            verification: Verification::Enabled,
            ca_file: None,
            certificate: None,
            http2: false,
            ecdh_curve: None,
            user_agent: "betterlite-test".to_owned(),
        },
    }
}

struct NoCapture;

impl ErrorCapture for NoCapture {
    fn capture<'body>(
        &'body self,
        _exchange: Exchange,
        _body: BodyChunks<'body>,
    ) -> Pin<Box<dyn Future<Output = CapturedBody> + Send + 'body>> {
        Box::pin(async {
            CapturedBody {
                retained: Vec::new(),
                preview: String::new(),
            }
        })
    }
}

fn http_transport() -> Arc<McpHttpTransport> {
    Arc::new(McpHttpTransport::new(&defaults(), &base(), Arc::new(NoCapture)).unwrap())
}

fn streamable(url: Url) -> (SdkTransport, MessageHandler<SdkError>) {
    let transport = http_transport();
    let cleanup = Arc::new(McpHttpClient::new(
        Arc::clone(&transport),
        TaskTracker::new(),
    ));
    let handler: MessageHandler<SdkError> = MessageHandler::default();
    let http = StreamableHttp::new(url, transport, cleanup, handler.clone());
    (SdkTransport::Streamable(http), handler)
}

fn legacy(url: Url) -> (SdkTransport, MessageHandler<SdkError>) {
    let handler: MessageHandler<SdkError> = MessageHandler::default();
    let transport = SdkTransport::LegacySse(LegacySse {
        url,
        http: http_transport(),
        handler: handler.clone(),
    });
    (transport, handler)
}

#[derive(Clone)]
struct RecordedRequest {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone)]
struct ServerState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    reply: Arc<dyn Fn(&RecordedRequest) -> Response + Send + Sync>,
}

struct TestServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    _tasks: JoinSet<()>,
}

impl TestServer {
    async fn start<Reply>(reply: Reply) -> Self
    where
        Reply: Fn(&RecordedRequest) -> Response + Send + Sync + 'static,
    {
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::default();
        let state = ServerState {
            requests: Arc::clone(&requests),
            reply: Arc::new(reply),
        };
        let app = Router::new().fallback(record).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            requests,
            _tasks: tasks,
        }
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.address)).unwrap()
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

async fn record(State(state): State<ServerState>, request: IncomingRequest) -> Response {
    let (parts, payload) = request.into_parts();
    let content = to_bytes(payload, BODY_LIMIT).await.unwrap_or_default();
    let recorded = RecordedRequest {
        method: parts.method,
        uri: parts.uri.to_string(),
        headers: parts.headers,
        body: content.to_vec(),
    };
    let response = (state.reply)(&recorded);
    state.requests.lock().unwrap().push(recorded);
    response
}

fn answer(status: u16, content_type: &'static str, body: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::from_u16(status).unwrap();
    if !content_type.is_empty() {
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    }
    response
}

fn relocated(status: u16, location: &'static str) -> Response {
    let mut response = answer(status, "text/plain", "moved");
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static(location));
    response
}

fn streamed(chunks: Vec<Bytes>) -> Response {
    let items: Vec<Result<Bytes, Infallible>> = chunks.into_iter().map(Ok).collect();
    let mut response = Response::new(Body::from_stream(futures_util::stream::iter(items)));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
}

fn request(id: i64, method: &str) -> TxJsonRpcMessage<RoleClient> {
    JsonRpcMessage::request(
        ClientRequest::CustomRequest(CustomRequest::new(method, None)),
        RequestId::Number(id),
    )
}

fn notification(method: &str) -> TxJsonRpcMessage<RoleClient> {
    JsonRpcMessage::notification(ClientNotification::CustomNotification(
        CustomNotification::new(method, None),
    ))
}

fn initialized() -> TxJsonRpcMessage<RoleClient> {
    JsonRpcMessage::notification(ClientNotification::InitializedNotification(
        InitializedNotification::default(),
    ))
}

async fn enter(transport: &mut SdkTransport) -> SdkStreams {
    transport.enter().await.unwrap()
}

async fn send(streams: &mut SdkStreams, item: TxJsonRpcMessage<RoleClient>) {
    <SdkStreams as Transport<RoleClient>>::send(streams, item)
        .await
        .unwrap();
}

async fn receive(streams: &mut SdkStreams) -> RxJsonRpcMessage<RoleClient> {
    <SdkStreams as Transport<RoleClient>>::receive(streams)
        .await
        .expect("a message arrives")
}

fn response_id(message: &RxJsonRpcMessage<RoleClient>) -> Option<RequestId> {
    match message {
        JsonRpcMessage::Response(response) => Some(response.id.clone()),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) | JsonRpcMessage::Error(_) => {
            None
        }
    }
}

fn error_of(message: &RxJsonRpcMessage<RoleClient>) -> (i32, String) {
    match message {
        JsonRpcMessage::Error(error) => (error.error.code.0, error.error.message.to_string()),
        JsonRpcMessage::Request(_)
        | JsonRpcMessage::Notification(_)
        | JsonRpcMessage::Response(_) => {
            panic!("expected a JSON-RPC error")
        }
    }
}

fn note_header(request: &RecordedRequest, name: &HeaderName) -> Option<String> {
    request
        .headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

#[rstest]
#[case::moved_post(307, Method::POST, true)]
#[case::permanent_post(308, Method::POST, true)]
#[case::found_post(302, Method::POST, false)]
#[case::see_other_post(303, Method::POST, false)]
#[case::moved_post_to_get(301, Method::POST, false)]
#[case::found_get(302, Method::GET, true)]
#[case::see_other_delete(303, Method::DELETE, false)]
fn the_redirect_method_rule_follows_httpx2(
    #[case] status: u16,
    #[case] method: Method,
    #[case] followed: bool,
) {
    let sent = Url::parse("http://example.com/mcp").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, HeaderValue::from_static("/moved"));
    let target = next_request_within_origin(&method, &sent, status, &headers);
    assert_eq!(target.is_some(), followed);
}

#[test]
fn a_cross_origin_location_is_not_followed() {
    let sent = Url::parse("http://example.com/mcp").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, HeaderValue::from_static("http://other.test/mcp"));
    assert!(next_request_within_origin(&Method::POST, &sent, 307, &headers).is_none());
}

#[test]
fn an_upgrade_to_https_on_the_default_port_is_followed() {
    let sent = Url::parse("http://example.com/mcp").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        LOCATION,
        HeaderValue::from_static("https://example.com/mcp"),
    );
    let target = next_request_within_origin(&Method::GET, &sent, 307, &headers).unwrap();
    assert_eq!(target.url.as_str(), "https://example.com/mcp");
}

#[test]
fn a_location_with_its_own_userinfo_is_not_followed() {
    let sent = Url::parse("http://example.com/mcp").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        LOCATION,
        HeaderValue::from_static("http://user:pass@example.com/mcp"),
    );
    assert!(next_request_within_origin(&Method::GET, &sent, 307, &headers).is_none());
}

#[tokio::test]
async fn a_307_post_is_followed_within_the_origin() {
    let server = TestServer::start(|request| {
        if request.uri == "/moved" {
            answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}",
            )
        } else {
            relocated(307, "/moved")
        }
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(7)));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].uri, "/moved");
}

#[tokio::test]
async fn a_302_on_a_post_is_not_followed_and_names_the_location() {
    let server = TestServer::start(|_| relocated(302, "/elsewhere")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32600);
    assert!(text.contains("/elsewhere"), "{text}");
    assert!(text.contains("not followed"), "{text}");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_twenty_first_redirect_is_not_followed() {
    let server = TestServer::start(|_| relocated(307, "/mcp")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32600);
    assert!(text.contains("not followed"), "{text}");
    assert_eq!(server.requests().len(), 21);
}

#[tokio::test]
async fn a_202_resolves_the_request_as_invalid() {
    let server = TestServer::start(|_| answer(202, "", "")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "server answered a request with 202 Accepted");
}

#[tokio::test]
async fn a_404_before_a_session_is_method_not_found() {
    let server = TestServer::start(|_| answer(404, "text/plain", "missing")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32601);
    assert_eq!(text, "Not Found");
}

#[tokio::test]
async fn a_404_after_a_session_is_session_terminated() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            let mut response = answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            );
            response
                .headers_mut()
                .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
            return response;
        }
        answer(404, "text/plain", "missing")
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    assert_eq!(
        response_id(&receive(&mut streams).await),
        Some(RequestId::Number(1))
    );
    send(&mut streams, request(2, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "Session terminated");
}

#[tokio::test]
async fn a_json_error_body_wins_over_the_status() {
    let server = TestServer::start(|_| {
        answer(
            400,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32602,\"message\":\"bad params\"}}",
        )
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(5, "tools/list")).await;
    let message = receive(&mut streams).await;
    let (code, text) = error_of(&message);
    assert_eq!(code, -32602);
    assert_eq!(text, "bad params");
    match &message {
        JsonRpcMessage::Error(error) => assert_eq!(error.id, Some(RequestId::Number(5))),
        JsonRpcMessage::Request(_)
        | JsonRpcMessage::Notification(_)
        | JsonRpcMessage::Response(_) => {
            panic!("expected a JSON-RPC error")
        }
    }
}

#[tokio::test]
async fn a_500_without_a_json_body_is_internal_error() {
    let server = TestServer::start(|_| answer(500, "text/plain", "boom")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32603);
    assert_eq!(text, "Server returned an error response");
}

#[tokio::test]
async fn an_unknown_content_type_is_invalid() {
    let server = TestServer::start(|_| answer(200, "text/plain", "text")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32600);
    assert_eq!(text, "Unexpected content type: text/plain");
}

#[tokio::test]
async fn the_protocol_version_header_is_cleared_for_initialize() {
    let server = TestServer::start(|_| {
        answer(
            200,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        )
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let SdkTransport::Streamable(http) = &transport else {
        panic!("expected the streamable transport")
    };
    http.set_protocol_version("2025-06-18");
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "tools/list")).await;
    send(&mut streams, request(1, "initialize")).await;
    let requests = server.requests();
    assert_eq!(
        note_header(&requests[0], &VERSION_HEADER).as_deref(),
        Some("2025-06-18")
    );
    assert_eq!(note_header(&requests[1], &VERSION_HEADER), None);
}

#[tokio::test]
async fn an_sse_response_rewrites_the_id_and_reconnects_with_last_event_id() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return streamed(vec![
                Bytes::from_static(b"event: message\nid: evt-1\ndata:\n\n"),
                Bytes::from_static(
                    b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
                ),
            ]);
        }
        streamed(vec![Bytes::from_static(
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"server-9\",\"result\":{}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let notification = receive(&mut streams).await;
    assert!(matches!(notification, JsonRpcMessage::Notification(_)));
    let response = receive(&mut streams).await;
    assert_eq!(response_id(&response), Some(RequestId::Number(7)));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(
        note_header(&requests[1], &LAST_EVENT_HEADER).as_deref(),
        Some("evt-1")
    );
}

#[tokio::test]
async fn an_sse_event_over_the_limit_fails_the_request() {
    let server = TestServer::start(|_| {
        let payload = format!("event: message\ndata: {}\n\n", "x".repeat(1024 * 1024 + 16));
        streamed(vec![Bytes::from(payload)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    let (code, text) = error_of(&receive(&mut streams).await);
    assert_eq!(code, -32000);
    assert!(text.contains("1048576"), "{text}");
}

#[tokio::test]
async fn the_session_id_is_kept_and_the_delete_is_sent_on_close() {
    let server = TestServer::start(|request| {
        if request.method == Method::DELETE {
            return answer(200, "", "");
        }
        let mut response = answer(
            200,
            "application/json",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
        );
        response
            .headers_mut()
            .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
        response
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    <SdkStreams as Transport<RoleClient>>::close(&mut streams)
        .await
        .unwrap();
    let requests = server.requests();
    let delete = requests.last().unwrap();
    assert_eq!(delete.method, Method::DELETE);
    assert_eq!(
        note_header(delete, &SESSION_HEADER).as_deref(),
        Some("session-1")
    );
}

#[tokio::test]
async fn a_get_stream_failure_reaches_the_handler() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST
            && request.body.starts_with(b"{\"jsonrpc\":\"2.0\",\"id\":1")
        {
            let mut response = answer(
                200,
                "application/json",
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}",
            );
            response
                .headers_mut()
                .insert(SESSION_HEADER, HeaderValue::from_static("session-1"));
            return response;
        }
        if request.method == Method::POST {
            return answer(202, "", "");
        }
        answer(500, "text/plain", "boom")
    })
    .await;
    let (mut transport, handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "initialize")).await;
    let _response = receive(&mut streams).await;
    send(&mut streams, initialized()).await;
    assert!(
        <SdkStreams as Transport<RoleClient>>::receive(&mut streams)
            .await
            .is_none()
    );
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::Http);
}

#[tokio::test]
async fn a_legacy_endpoint_on_another_origin_is_refused() {
    let server = TestServer::start(|_| {
        streamed(vec![Bytes::from_static(
            b"event: endpoint\ndata: http://127.0.0.1:1/messages\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = legacy(server.url("/sse"));
    let error = transport.enter().await.err().unwrap();
    assert_eq!(
        error.to_string(),
        "Endpoint origin does not match connection origin: http://127.0.0.1:1/messages"
    );
}

#[tokio::test]
async fn keep_alives_are_skipped_and_a_bad_message_is_reported() {
    let server = TestServer::start(|_| {
        streamed(vec![Bytes::from_static(
            b"event: endpoint\ndata: /messages\n\nevent: message\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\nevent: message\ndata: not-json\n\n",
        )])
    })
    .await;
    let (mut transport, handler) = legacy(server.url("/sse"));
    let mut streams = enter(&mut transport).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(1)));
    assert!(
        <SdkStreams as Transport<RoleClient>>::receive(&mut streams)
            .await
            .is_none()
    );
    let recorded = handler.recorded.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.kind(), StreamFailureKind::ValueError);
}

#[tokio::test]
async fn a_legacy_post_goes_to_the_endpoint_through_the_origin_rule() {
    let server = TestServer::start(|request| {
        if request.method == Method::GET {
            return streamed(vec![Bytes::from_static(
                b"event: endpoint\ndata: /messages\n\n",
            )]);
        }
        if request.uri == "/messages" {
            return relocated(307, "/messages2");
        }
        answer(200, "", "")
    })
    .await;
    let (mut transport, _handler) = legacy(server.url("/sse"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, notification("notifications/initialized")).await;
    let requests = server.requests();
    let posts: Vec<&RecordedRequest> = requests
        .iter()
        .filter(|request| request.method == Method::POST)
        .collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].uri, "/messages");
    assert_eq!(posts[1].uri, "/messages2");
}

#[tokio::test]
async fn a_legacy_post_with_an_error_status_fails_the_send() {
    let server = TestServer::start(|request| {
        if request.method == Method::GET {
            return streamed(vec![Bytes::from_static(
                b"event: endpoint\ndata: /messages\n\n",
            )]);
        }
        answer(400, "text/plain", "denied")
    })
    .await;
    let (mut transport, _handler) = legacy(server.url("/sse"));
    let mut streams = enter(&mut transport).await;
    let error = <SdkStreams as Transport<RoleClient>>::send(
        &mut streams,
        notification("notifications/initialized"),
    )
    .await
    .err()
    .unwrap();
    match error {
        SdkError::Http(source) => assert!(source.to_string().contains("400"), "{source}"),
        other => panic!("expected an HTTP error, got {other}"),
    }
}

struct UnusedInit;

impl InitializeOutcome for UnusedInit {
    fn protocol_version(&self) -> &str {
        "2025-06-18"
    }

    fn instructions(&self) -> Option<&str> {
        None
    }
}

struct UnusedSession;

impl ClientSession for UnusedSession {
    type Error = io::Error;
    type InitializeResult = UnusedInit;

    fn initialize(&mut self) -> impl Future<Output = Result<UnusedInit, io::Error>> + Send {
        ready(Ok(UnusedInit))
    }

    fn send_initialize(
        &mut self,
        _request: InitializeRequest,
    ) -> impl Future<Output = Result<UnusedInit, io::Error>> + Send {
        ready(Ok(UnusedInit))
    }

    fn adopt(&mut self, _result: &UnusedInit) {}

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), io::Error>> + Send {
        ready(Ok(()))
    }
}

struct UnusedFactory;

impl StdioSessionFactory for UnusedFactory {
    type Session = UnusedSession;

    fn build(&self, _streams: StdioStreams, _options: SessionOptions<io::Error>) -> UnusedSession {
        UnusedSession
    }
}

#[tokio::test]
async fn a_stdio_round_trip_through_the_async_rw_transport() {
    let script = "while IFS= read -r line; do printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'; done";
    let runtime = StdioRuntime {
        factory: Arc::new(UnusedFactory),
    };
    let stdio = runtime.stdio_client(StdioServerParameters {
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script.to_owned()],
        env: BTreeMap::new(),
        defaults: BTreeMap::new(),
    });
    let mut transport = SdkTransport::Stdio(stdio);
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "ping")).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(1)));
    drop(streams);
    transport.exit().await.unwrap();
}

#[tokio::test]
async fn entering_a_streamable_transport_twice_is_refused() {
    let server = TestServer::start(|_| answer(200, "application/json", "{}")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let _streams = enter(&mut transport).await;
    let error = transport.enter().await.err().unwrap();
    assert!(error.to_string().contains("already open"), "{error}");
}
