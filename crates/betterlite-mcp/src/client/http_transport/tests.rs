#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aws_credential_types::Credentials;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_smithy_async::time::SharedTimeSource;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request as IncomingRequest, State};
use axum::response::Response;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::mcp_sigv4::McpSigV4Auth;
use betterlite_http::mcp_sigv4::cache::CredentialCache;
use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue, LOCATION, WWW_AUTHENTICATE};
use http::{Method, StatusCode};
use rstest::rstest;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::task::TaskTracker;
use url::Url;

use super::capture::{BodyChunks, CapturedBody, ErrorCapture, Exchange};
use super::error::McpHttpError;
use super::request::McpHttpRequest;
use super::send::{HttpBase, McpHttpTransport};
use crate::client::cleanup::http_client::{ClientError, McpHttpClient, Transport as _};
use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::sessions::factory::{
    AuthSource, HttpClientConfig, RedirectGuard, RequestCredential, TlsSettings, TlsVerification,
};
use crate::credentials::{CredentialError, Request as CredentialRequest, ResolvedAuth};

const CA_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/http_transport/fixtures/ca.pem"
);
const SERVER_BODY: &str = "server body";
const RETAINED: &[u8] = b"retained prefix";
const BODY_LIMIT: usize = 0x0010_0000;

fn defaults() -> HttpClientConfig {
    HttpClientConfig {
        headers: BTreeMap::new(),
        timeout: Duration::from_secs(5),
        tls: TlsSettings {
            verification: TlsVerification::Enabled,
            ca_file: None,
        },
        auth: AuthSource::None,
        credential: None,
        follow_redirects: true,
        redirect_guard: None,
        capture_error_responses: true,
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

fn build(config: &HttpClientConfig, capture: Arc<dyn ErrorCapture>) -> McpHttpTransport {
    McpHttpTransport::new(config, &base(), capture).unwrap()
}

fn request(method: Method, url: Url) -> McpHttpRequest {
    McpHttpRequest {
        method,
        url,
        headers: HeaderMap::new(),
        body: Bytes::new(),
        read_timeout: None,
    }
}

fn sigv4_signer() -> Arc<McpSigV4Auth> {
    let clock = SharedTimeSource::default();
    let credentials = Credentials::new(
        "AKIDEXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        None,
        None,
        "fixture",
    );
    Arc::new(McpSigV4Auth {
        requires_request_body: true,
        service_name: "bedrock-agentcore".to_owned(),
        region_name: "us-east-1".to_owned(),
        clock: clock.clone(),
        cache: CredentialCache {
            clock,
            provider: SharedCredentialsProvider::new(credentials),
            current: Mutex::new(None),
        },
    })
}

fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
    let mut response = Response::new(Body::from(body.to_owned()));
    *response.status_mut() = StatusCode::from_u16(status).unwrap();
    for (name, value) in headers {
        response.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    response
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    state
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(recorded);
    response
}

#[derive(Default)]
struct RecordingCapture {
    calls: Mutex<Vec<Exchange>>,
}

impl RecordingCapture {
    fn count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn exchanges(&self) -> Vec<Exchange> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ErrorCapture for RecordingCapture {
    fn capture<'body>(
        &'body self,
        exchange: Exchange,
        mut body: BodyChunks<'body>,
    ) -> Pin<Box<dyn Future<Output = CapturedBody> + Send + 'body>> {
        Box::pin(async move {
            let mut read = 0_usize;
            while let Some(chunk) = body.next_chunk().await {
                match chunk {
                    Ok(bytes) => read = read.saturating_add(bytes.len()),
                    Err(_) => break,
                }
            }
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(exchange);
            CapturedBody {
                retained: RETAINED.to_vec(),
                preview: format!("{read} bytes"),
            }
        })
    }
}

struct MergingAuth;

impl ResolvedAuth for MergingAuth {
    fn authenticate(
        &self,
        mut request: CredentialRequest,
    ) -> Result<CredentialRequest, CredentialError> {
        let seen = request.headers.get("x-config").cloned().unwrap_or_default();
        request.headers.insert("x-credential".to_owned(), seen);
        Ok(request)
    }

    fn close(&self) {}
}

struct FailingAuth;

impl ResolvedAuth for FailingAuth {
    fn authenticate(
        &self,
        _request: CredentialRequest,
    ) -> Result<CredentialRequest, CredentialError> {
        Err(CredentialError::AuthenticationFailed)
    }

    fn close(&self) {}
}

struct ScriptedAuth {
    refresh: bool,
    attempts: AtomicUsize,
    after: AtomicUsize,
}

impl ScriptedAuth {
    fn new(refresh: bool) -> Self {
        Self {
            refresh,
            attempts: AtomicUsize::new(0),
            after: AtomicUsize::new(0),
        }
    }
}

impl ResolvedAuth for ScriptedAuth {
    fn authenticate(
        &self,
        mut request: CredentialRequest,
    ) -> Result<CredentialRequest, CredentialError> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        request.headers.insert(
            "authorization".to_owned(),
            format!("Bearer token-{attempt}"),
        );
        Ok(request)
    }

    fn close(&self) {}

    fn after_unauthorized(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        self.after.fetch_add(1, Ordering::SeqCst);
        let refresh = self.refresh;
        Box::pin(async move { refresh })
    }
}

#[tokio::test]
async fn config_headers_are_merged_with_the_request_winning() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let mut config = defaults();
    config.headers = BTreeMap::from([
        ("x-config".to_owned(), "config".to_owned()),
        ("X-Shared".to_owned(), "config".to_owned()),
    ]);
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let mut request = request(Method::GET, server.url("/mcp"));
    request.headers.insert(
        HeaderName::from_static("x-shared"),
        HeaderValue::from_static("request"),
    );
    request.headers.insert(
        HeaderName::from_static("x-request"),
        HeaderValue::from_static("request"),
    );
    let _response = transport.send(request, false).await.unwrap();
    let recorded = server.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].method, Method::GET);
    assert_eq!(recorded[0].uri, "/mcp");
    let headers = &recorded[0].headers;
    assert_eq!(headers.get("x-config").unwrap().to_str().unwrap(), "config");
    assert_eq!(
        headers.get("x-shared").unwrap().to_str().unwrap(),
        "request"
    );
    assert_eq!(
        headers.get("x-request").unwrap().to_str().unwrap(),
        "request"
    );
}

#[tokio::test]
async fn a_resolved_credential_is_applied_to_the_merged_request() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let mut config = defaults();
    config.headers = BTreeMap::from([("x-config".to_owned(), "config-value".to_owned())]);
    config.credential = Some(RequestCredential::Resolved(Arc::new(MergingAuth)));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let _response = transport
        .send(request(Method::POST, server.url("/mcp")), false)
        .await
        .unwrap();
    let recorded = server.requests();
    assert_eq!(
        recorded[0]
            .headers
            .get("x-credential")
            .unwrap()
            .to_str()
            .unwrap(),
        "config-value"
    );
}

#[tokio::test]
async fn a_failing_credential_stops_before_sending() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let mut config = defaults();
    config.credential = Some(RequestCredential::Resolved(Arc::new(FailingAuth)));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let error = transport
        .send(request(Method::GET, server.url("/mcp")), false)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        McpHttpError::Credential(CredentialError::AuthenticationFailed)
    ));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn sigv4_signs_the_body_bytes() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let mut config = defaults();
    config.credential = Some(RequestCredential::AwsSigV4(sigv4_signer()));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let body = Bytes::from_static(b"{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\"}");
    let mut request = request(Method::POST, server.url("/mcp"));
    request.body = body.clone();
    let _response = transport.send(request, false).await.unwrap();
    let recorded = server.requests();
    let headers = &recorded[0].headers;
    let expected = hex::encode(Sha256::digest(&body));
    assert_eq!(
        headers
            .get("x-amz-content-sha256")
            .unwrap()
            .to_str()
            .unwrap(),
        expected
    );
    let authorization = headers.get("authorization").unwrap().to_str().unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256"));
    assert_eq!(recorded[0].body, body.to_vec());
}

#[tokio::test]
async fn sigv4_never_retries_a_401() {
    let server = TestServer::start(|_| {
        answer(
            401,
            &[("www-authenticate", "Bearer error=\"invalid_token\"")],
            "denied",
        )
    })
    .await;
    let mut config = defaults();
    config.credential = Some(RequestCredential::AwsSigV4(sigv4_signer()));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let response = transport
        .send(request(Method::POST, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, 401);
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn a_307_is_returned_unfollowed_with_its_location() {
    let server = TestServer::start(|_| answer(307, &[("location", "/elsewhere")], "moved")).await;
    let transport = build(&defaults(), Arc::new(RecordingCapture::default()));
    let response = transport
        .send(request(Method::GET, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, 307);
    assert_eq!(
        response.headers.get(LOCATION).unwrap().to_str().unwrap(),
        "/elsewhere"
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn redirect_guard_drops_a_moved_credential_on_another_origin() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let mut config = defaults();
    config.headers = BTreeMap::from([("x-api-key".to_owned(), "secret".to_owned())]);
    config.redirect_guard = RedirectGuard::new("https://mcp.example.com/mcp", Some("X-API-Key"));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let _response = transport
        .send(request(Method::GET, server.url("/mcp")), false)
        .await
        .unwrap();
    assert!(!server.requests()[0].headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn redirect_guard_keeps_the_credential_on_the_configured_origin() {
    let server = TestServer::start(|_| answer(200, &[], SERVER_BODY)).await;
    let configured = server.url("/mcp");
    let mut config = defaults();
    config.headers = BTreeMap::from([("x-api-key".to_owned(), "secret".to_owned())]);
    config.redirect_guard = RedirectGuard::new(configured.as_str(), Some("X-API-Key"));
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let _response = transport
        .send(request(Method::GET, server.url("/other")), false)
        .await
        .unwrap();
    assert_eq!(
        server.requests()[0]
            .headers
            .get("x-api-key")
            .unwrap()
            .to_str()
            .unwrap(),
        "secret"
    );
}

#[tokio::test]
async fn redirect_guard_keeps_the_credential_on_an_http_to_https_upgrade() {
    let mut config = defaults();
    config.headers = BTreeMap::from([("x-api-key".to_owned(), "secret".to_owned())]);
    config.redirect_guard = RedirectGuard::new("http://127.0.0.1:8080/mcp", Some("X-API-Key"));
    let _transport = build(&config, Arc::new(RecordingCapture::default()));
    let upgraded = request(
        Method::GET,
        Url::parse("https://127.0.0.1:8080/mcp").unwrap(),
    );
    let prepared = super::request::prepare(&config, &upgraded).await.unwrap();
    assert_eq!(
        prepared.headers.get("x-api-key").unwrap().to_str().unwrap(),
        "secret"
    );
    let other_port = request(
        Method::GET,
        Url::parse("https://127.0.0.1:9090/mcp").unwrap(),
    );
    let prepared_other = super::request::prepare(&config, &other_port).await.unwrap();
    assert!(!prepared_other.headers.contains_key("x-api-key"));
}

#[rstest]
#[case::server_error(500)]
#[case::unauthorized(401)]
#[case::success(200)]
#[case::redirect(302)]
#[tokio::test]
async fn capture_runs_once_for_every_error_response(#[case] status: u16) {
    let server = TestServer::start(move |_| answer(status, &[], SERVER_BODY)).await;
    let capture = Arc::new(RecordingCapture::default());
    let transport = build(&defaults(), Arc::<RecordingCapture>::clone(&capture));
    let mut response = transport
        .send(request(Method::GET, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, status);
    if status >= 400 {
        assert_eq!(capture.count(), 1);
        assert_eq!(response.body.read_all().await.unwrap(), RETAINED.to_vec());
        let record = response.capture.as_ref().unwrap();
        assert_eq!(record.preview, format!("{} bytes", SERVER_BODY.len()));
        assert_eq!(record.exchange.status, status);
        assert_eq!(record.exchange.url, server.url("/mcp"));
    } else {
        assert_eq!(capture.count(), 0);
        assert_eq!(
            response.body.read_all().await.unwrap(),
            SERVER_BODY.as_bytes().to_vec()
        );
        assert!(response.capture.is_none());
    }
}

#[tokio::test]
async fn capture_is_skipped_when_the_config_disables_it() {
    let server = TestServer::start(|_| answer(500, &[], SERVER_BODY)).await;
    let capture = Arc::new(RecordingCapture::default());
    let mut config = defaults();
    config.capture_error_responses = false;
    let transport = build(&config, Arc::<RecordingCapture>::clone(&capture));
    let mut response = transport
        .send(request(Method::GET, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, 500);
    assert_eq!(capture.count(), 0);
    assert_eq!(
        response.body.read_all().await.unwrap(),
        SERVER_BODY.as_bytes().to_vec()
    );
    assert!(response.capture.is_none());
}

#[tokio::test]
async fn a_401_is_sent_once_more_when_the_credential_refreshes() {
    let server = TestServer::start(|_| {
        answer(
            401,
            &[("www-authenticate", "Bearer error=\"invalid_token\"")],
            "denied",
        )
    })
    .await;
    let auth = Arc::new(ScriptedAuth::new(true));
    let credential: Arc<dyn ResolvedAuth> = Arc::<ScriptedAuth>::clone(&auth);
    let mut config = defaults();
    config.credential = Some(RequestCredential::Resolved(credential));
    let capture = Arc::new(RecordingCapture::default());
    let transport = build(&config, Arc::<RecordingCapture>::clone(&capture));
    let response = transport
        .send(request(Method::POST, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, 401);
    assert_eq!(
        response.capture.as_ref().unwrap().preview,
        format!("{} bytes", "denied".len())
    );
    assert_eq!(auth.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(auth.after.load(Ordering::SeqCst), 1);
    assert_eq!(capture.count(), 2);
    let recorded = server.requests();
    let exchanges = capture.exchanges();
    assert_eq!(recorded.len(), 2);
    assert_eq!(exchanges.len(), 2);
    assert_refreshed_authorization(&recorded[0], &recorded[1], &exchanges[1]);
}

fn assert_refreshed_authorization(
    first: &RecordedRequest,
    second: &RecordedRequest,
    exchange: &Exchange,
) {
    assert_eq!(authorization(&first.headers), "Bearer token-0");
    assert_eq!(authorization(&second.headers), "Bearer token-1");
    assert_eq!(authorization(&exchange.request_headers), "Bearer token-1");
}

fn authorization(headers: &HeaderMap) -> &str {
    headers.get("authorization").unwrap().to_str().unwrap()
}

#[tokio::test]
async fn a_401_is_returned_at_once_when_the_credential_declines() {
    let server = TestServer::start(|_| answer(401, &[], "denied")).await;
    let auth = Arc::new(ScriptedAuth::new(false));
    let credential: Arc<dyn ResolvedAuth> = Arc::<ScriptedAuth>::clone(&auth);
    let mut config = defaults();
    config.credential = Some(RequestCredential::Resolved(credential));
    let capture = Arc::new(RecordingCapture::default());
    let transport = build(&config, Arc::<RecordingCapture>::clone(&capture));
    let response = transport
        .send(request(Method::POST, server.url("/mcp")), false)
        .await
        .unwrap();
    assert_eq!(response.status, 401);
    assert_eq!(auth.after.load(Ordering::SeqCst), 1);
    assert_eq!(capture.count(), 1);
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn status_error_keeps_www_authenticate_and_the_capture() {
    let server = TestServer::start(|_| {
        answer(
            401,
            &[("www-authenticate", "Bearer realm=\"mcp\"")],
            "denied body",
        )
    })
    .await;
    let capture = Arc::new(RecordingCapture::default());
    let transport = Arc::new(build(&defaults(), Arc::<RecordingCapture>::clone(&capture)));
    let client = McpHttpClient::new(transport, TaskTracker::new());
    let error = client
        .send(request(Method::POST, server.url("/mcp")), false)
        .await
        .err()
        .unwrap();
    let ClientError::Transport(transport_error) = error else {
        panic!("expected a transport error");
    };
    let McpHttpError::Status(head) = transport_error.as_ref() else {
        panic!("expected a status error");
    };
    assert_eq!(head.status, 401);
    assert_eq!(
        head.headers
            .get(WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer realm=\"mcp\""
    );
    assert_eq!(
        head.capture.as_ref().unwrap().preview,
        format!("{} bytes", "denied body".len())
    );
    assert_eq!(capture.count(), 1);
}

async fn stalling_server() -> (SocketAddr, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\n\r\n")
            .await
            .unwrap();
        pending::<()>().await;
    });
    (address, tasks)
}

#[tokio::test(start_paused = true)]
async fn a_body_read_after_the_head_times_out_on_paused_time() {
    let (address, _tasks) = stalling_server().await;
    let mut config = defaults();
    config.timeout = Duration::from_secs(3);
    let transport = build(&config, Arc::new(RecordingCapture::default()));
    let url = Url::parse(&format!("http://{address}/mcp")).unwrap();
    let started = Instant::now();
    let mut response = transport
        .send(request(Method::GET, url), false)
        .await
        .unwrap();
    let error = response.body.read_all().await.err().unwrap();
    assert!(matches!(error, McpHttpError::BodyTimeout(_)));
    assert_eq!(started.elapsed(), Duration::from_secs(3));
}

#[rstest]
#[case::disabled(TlsVerification::Disabled, None)]
#[case::ca_file(TlsVerification::CaFile, Some(CA_FILE))]
fn tls_disabled_and_a_ca_file_build_a_client(
    #[case] verification: TlsVerification,
    #[case] ca_file: Option<&str>,
) {
    let mut config = defaults();
    config.tls = TlsSettings {
        verification,
        ca_file: ca_file.map(str::to_owned),
    };
    let _transport = build(&config, Arc::new(RecordingCapture::default()));
}

#[test]
fn tls_ca_file_that_cannot_be_read_fails_the_build() {
    let mut config = defaults();
    config.tls = TlsSettings {
        verification: TlsVerification::CaFile,
        ca_file: Some("/nonexistent/betterlite-ca.pem".to_owned()),
    };
    let error = McpHttpTransport::new(&config, &base(), Arc::new(RecordingCapture::default()))
        .err()
        .unwrap();
    assert!(matches!(error, McpHttpError::Tls(_)));
}
