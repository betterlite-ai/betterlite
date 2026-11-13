#![cfg(test)]

mod streamable;

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

use super::context::{SdkStreams, SdkTransport, StdioContext};
use super::error::SdkError;
use super::legacy_sse::LegacySse;
use super::origin::next_request_within_origin;
use super::streamable::{StreamableCore, StreamableHttp};

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

fn cut_stream(first: Bytes) -> Response {
    let items = vec![
        Ok::<Bytes, std::io::Error>(first),
        Err(std::io::Error::other("cut")),
    ];
    let mut response = Response::new(Body::from_stream(futures_util::stream::iter(items)));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
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
fn an_upgrade_with_an_explicit_port_is_not_followed() {
    let sent = Url::parse("http://example.com:8080/mcp").unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        LOCATION,
        HeaderValue::from_static("https://example.com/mcp"),
    );
    assert!(next_request_within_origin(&Method::GET, &sent, 307, &headers).is_none());
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

#[rstest]
#[case::new_userinfo("http://example.com/mcp", "http://user:pass@example.com/mcp", false)]
#[case::password_without_user("http://example.com/mcp", "http://:pass@example.com/mcp", false)]
#[case::user_without_password("http://example.com/mcp", "http://u@example.com/mcp", false)]
#[case::other_user("http://a:x@example.com/mcp", "http://b:x@example.com/mcp", false)]
#[case::other_password("http://a:x@example.com/mcp", "http://a:y@example.com/mcp", false)]
#[case::same_userinfo("http://a:x@example.com/mcp", "http://a:x@example.com/mcp", true)]
#[case::no_userinfo("http://a:x@example.com/mcp", "http://example.com/mcp", true)]
fn the_userinfo_rule_follows_httpx2(
    #[case] sent: &str,
    #[case] location: &'static str,
    #[case] followed: bool,
) {
    let sent = Url::parse(sent).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, HeaderValue::from_static(location));
    let target = next_request_within_origin(&Method::GET, &sent, 307, &headers);
    if followed {
        assert!(target.is_some(), "{sent} should follow {location}");
    } else {
        assert!(target.is_none(), "{sent} must not follow {location}");
    }
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

#[tokio::test]
async fn a_data_only_event_is_a_message() {
    let server = TestServer::start(|_| {
        streamed(vec![Bytes::from_static(
            b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}

#[tokio::test]
async fn a_cut_sse_body_reconnects_with_last_event_id() {
    let server = TestServer::start(|request| {
        if request.method == Method::POST {
            return cut_stream(Bytes::from_static(
                b"event: message\nid: cut-1\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"x\"}}\n\n",
            ));
        }
        streamed(vec![Bytes::from_static(
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"server-9\",\"result\":{}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
    let response = receive(&mut streams).await;
    assert_eq!(response_id(&response), Some(RequestId::Number(7)));
    let requests = server.requests();
    assert_eq!(requests[1].method, Method::GET);
    assert_eq!(
        note_header(&requests[1], &LAST_EVENT_HEADER).as_deref(),
        Some("cut-1")
    );
}

#[tokio::test]
async fn a_comment_line_does_not_count_toward_the_event_limit() {
    let prefix = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"";
    let suffix = "\"}}";
    let padding = 524_288 - prefix.len() - suffix.len();
    let json = format!("{prefix}{}{suffix}", "x".repeat(padding));
    let comment = format!(": {}", "c".repeat(600_000));
    let server = TestServer::start(move |_| {
        let event = format!("{comment}\nevent: message\ndata: {json}\n\n");
        streamed(vec![Bytes::from(event)])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}

#[tokio::test]
async fn a_cr_delimited_event_at_the_size_limit_is_accepted() {
    let prefix = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\",\"data\":\"";
    let suffix = "\"}}";
    let padding = 1_048_556 - prefix.len() - suffix.len();
    let json = format!("{prefix}{}{suffix}", "x".repeat(padding));
    let server = TestServer::start(move |_| {
        streamed(vec![Bytes::from(format!(
            "event: message\rdata: {json}\r\r"
        ))])
    })
    .await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(7, "tools/list")).await;
    assert!(matches!(
        receive(&mut streams).await,
        JsonRpcMessage::Notification(_)
    ));
}

#[tokio::test]
async fn a_second_close_sends_no_delete() {
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
    transport.exit().await.unwrap();
    let deletes = server
        .requests()
        .into_iter()
        .filter(|request| request.method == Method::DELETE)
        .count();
    assert_eq!(deletes, 1);
}

#[tokio::test]
async fn a_data_only_legacy_event_is_a_message() {
    let server = TestServer::start(|_| {
        streamed(vec![Bytes::from_static(
            b"event: endpoint\ndata: /messages\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n",
        )])
    })
    .await;
    let (mut transport, _handler) = legacy(server.url("/sse"));
    let mut streams = enter(&mut transport).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(1)));
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
    let mut transport = SdkTransport::Stdio(StdioContext(stdio));
    let mut streams = enter(&mut transport).await;
    send(&mut streams, request(1, "ping")).await;
    let message = receive(&mut streams).await;
    assert_eq!(response_id(&message), Some(RequestId::Number(1)));
    drop(streams);
    transport.exit().await.unwrap();
}

#[test]
fn the_reconnection_delay_defaults_to_one_second() {
    assert_eq!(
        StreamableCore::retry_delay(None),
        std::time::Duration::from_millis(1000)
    );
    assert_eq!(
        StreamableCore::retry_delay(Some(25)),
        std::time::Duration::from_millis(25)
    );
}

#[tokio::test]
async fn entering_a_streamable_transport_twice_is_refused() {
    let server = TestServer::start(|_| answer(200, "application/json", "{}")).await;
    let (mut transport, _handler) = streamable(server.url("/mcp"));
    let _streams = enter(&mut transport).await;
    let error = transport.enter().await.err().unwrap();
    assert!(error.to_string().contains("already open"), "{error}");
}
