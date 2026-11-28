#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::websocket_client::connect;
use betterlite_http::websocket_client::request::{Keepalive, WebSocketRequest};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use http::StatusCode;
use rstest::rstest;
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use url::Url;

use crate::api::realtime_backend_relay::source::{RealtimeBackendSource, RelayReceiveError};
use crate::api::realtime_backend_transport::dial::{
    ConnectedBackend, DialRequest, RealtimeSsl, SizeLimitedSource, open_realtime_backend_ws,
};
use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_connection_plan::{ProbeConnection, RealtimeProbeDialer};
use crate::api::realtime_contracts::close::{CloseFrame, ConnectionClosed, backend_close_from};
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::transport::RealtimeMessage;

use super::WebSocketDialer;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN_TIMEOUT: Duration = Duration::from_millis(100);
const SERVER_SILENCE: Duration = Duration::from_millis(200);
const MAX_SIZE: usize = 8;
const AT_LIMIT_FRAME: usize = 9;
const OVER_LIMIT_FRAME: usize = 64;
const ATTEMPTS: usize = 3;

// the upgrade callback: it records the request's headers when the test asks and rejects when it asks
struct UpgradeRecorder {
    seen: Option<Arc<Mutex<Vec<(String, String)>>>>,
    rejection: Option<StatusCode>,
}

impl Callback for UpgradeRecorder {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        if let Some(seen) = &self.seen {
            let mut recorded = seen.lock().expect("recorded headers");
            for (name, value) in request.headers() {
                recorded.push((
                    name.as_str().to_owned(),
                    value.to_str().expect("header text").to_owned(),
                ));
            }
        }
        match self.rejection {
            Some(status) => Err(Response::builder()
                .status(status)
                .body(Some("rejected".to_owned()))
                .expect("rejection")),
            None => Ok(response),
        }
    }
}

const fn web_dialer(proxy: Arc<ProxyPolicy>) -> WebSocketDialer {
    WebSocketDialer { proxy }
}

fn no_proxy() -> Arc<ProxyPolicy> {
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy"))
}

const fn request<'request>(
    url: &'request str,
    ssl: &'request RealtimeSsl,
    headers: &'request HeaderMap,
    max_size: usize,
) -> DialRequest<'request> {
    DialRequest {
        url,
        headers,
        ssl,
        max_size,
        open_timeout: OPEN_TIMEOUT,
    }
}

const fn probe_connection(url: String) -> ProbeConnection {
    ProbeConnection {
        url,
        headers: Headers::new(),
        ssl: RealtimeSsl::Disabled,
        max_size: 1024,
    }
}

// a server that answers each upgrade attempt with the same status, or holds it open until the open timeout fires
async fn serve_attempts(
    listener: TcpListener,
    attempts: Arc<AtomicUsize>,
    rejection: Option<StatusCode>,
) {
    let mut held = Vec::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        attempts.fetch_add(1, Ordering::SeqCst);
        match rejection {
            Some(status) => {
                drop(
                    tokio_tungstenite::accept_hdr_async(
                        stream,
                        UpgradeRecorder {
                            seen: None,
                            rejection: Some(status),
                        },
                    )
                    .await,
                );
            }
            None => {
                held.push(stream);
                assert!(held.len() <= ATTEMPTS);
            }
        }
    }
}

async fn serve_frames(listener: TcpListener, frames: Vec<Message>) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    for frame in frames {
        socket.send(frame).await.expect("frame");
    }
    pending::<()>().await;
}

async fn serve_close_after(listener: TcpListener, first: Option<Message>) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    if let Some(frame) = first {
        socket.send(frame).await.expect("frame");
    }
    let read = socket.next().await.expect("close").expect("frame");
    let Message::Close(Some(close)) = read else {
        panic!("expected a close frame");
    };
    (u16::from(close.code), close.reason.as_str().to_owned())
}

async fn serve_silent(listener: TcpListener) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    sleep(SERVER_SILENCE).await;
    let ping = socket.next().await.expect("ping").expect("frame");
    assert!(matches!(ping, Message::Ping(_)));
    let read = socket.next().await.expect("close").expect("frame");
    let Message::Close(Some(close)) = read else {
        panic!("expected a close frame");
    };
    (u16::from(close.code), close.reason.as_str().to_owned())
}

async fn serve_end(listener: TcpListener) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    drop(accept_async(stream).await);
}

// a server that reads the first byte of each attempt and drops the socket, so a TLS ClientHello shows itself
async fn record_first_byte(listener: TcpListener, seen: Arc<Mutex<Vec<u8>>>) {
    loop {
        let (mut stream, _peer) = listener.accept().await.expect("accept");
        let mut byte = [0_u8; 1];
        if let Ok(read) = stream.read(&mut byte).await {
            seen.lock().expect("seen").extend_from_slice(&byte[..read]);
        }
    }
}

// a server that records the upgrade's headers before it answers the handshake
async fn serve_with_headers(listener: TcpListener, seen: Arc<Mutex<Vec<(String, String)>>>) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    drop(
        tokio_tungstenite::accept_hdr_async(
            stream,
            UpgradeRecorder {
                seen: Some(seen),
                rejection: None,
            },
        )
        .await,
    );
}

// a server that reads the three frames the sender writes: text, binary and the normal close
async fn serve_sent_frames(listener: TcpListener) -> (Message, Message, Message) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    let text = socket.next().await.expect("text").expect("frame");
    let binary = socket.next().await.expect("binary").expect("frame");
    let close = socket.next().await.expect("close").expect("frame");
    (text, binary, close)
}

// the backend the dialer opens against a server, with no proxy and no keepalive
async fn dialed_backend(address: SocketAddr, max_size: usize) -> ConnectedBackend {
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, max_size)),
    )
    .await
    .expect("deadline")
    .expect("opened")
}

async fn dialed_source(address: SocketAddr, max_size: usize) -> Box<dyn RealtimeBackendSource> {
    dialed_backend(address, max_size)
        .await
        .source
        .expect("source")
}

fn expect_closed(
    outcome: Result<RealtimeMessage, RelayReceiveError>,
    failure: &str,
    arrival: &str,
) -> ConnectionClosed {
    match outcome {
        Err(RelayReceiveError::Closed(closed)) => closed,
        Err(_error) => panic!("{failure}"),
        Ok(_message) => panic!("{arrival}"),
    }
}

fn assert_oversized_close(closed: &ConnectionClosed, max_size: usize) {
    assert_eq!(
        closed.received,
        Some(CloseFrame {
            code: 1009,
            reason: "message too big".to_owned(),
        })
    );
    let expected = format!("the realtime backend message exceeds the {max_size}-byte limit");
    assert_eq!(closed.message, expected);
}

fn custom_headers(recorded: &[(String, String)]) -> Vec<(&str, &str)> {
    recorded
        .iter()
        .filter(|(name, _value)| {
            name.as_str() == "x-request-id" || name.as_str() == "authorization"
        })
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

// the three frames a normal session writes, as the server read them
fn assert_sent_frames(text: &Message, binary: &Message, close: &Message) {
    assert_eq!(text, &Message::Text(Utf8Bytes::from("hello")));
    assert_eq!(binary, &Message::Binary(Bytes::from_static(b"raw")));
    let Message::Close(Some(ref frame)) = *close else {
        panic!("expected a close frame");
    };
    assert_eq!(u16::from(frame.code), 1000);
    assert_eq!(frame.reason.as_str(), "");
}

#[tokio::test]
async fn dialer_answers_a_rejected_upgrade_with_the_status_after_one_attempt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_attempts(
        listener,
        Arc::clone(&attempts),
        Some(StatusCode::UNAUTHORIZED),
    ));
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the rejected upgrade opened"),
    };
    assert_eq!(
        error,
        DialError::InvalidStatus {
            status_code: 401,
            message: "server rejected WebSocket connection: HTTP 401".to_owned(),
        }
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    tasks.abort_all();
}

#[tokio::test]
async fn dialer_retries_a_hung_upgrade_three_times() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_attempts(listener, Arc::clone(&attempts), None));
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the hung upgrade opened"),
    };
    assert_eq!(
        error,
        DialError::Timeout {
            message: "timed out during opening handshake".to_owned(),
        }
    );
    assert_eq!(attempts.load(Ordering::SeqCst), ATTEMPTS);
    tasks.abort_all();
}

#[tokio::test]
async fn dialer_rejects_an_ssl_argument_on_a_ws_url_without_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Shared(Verification::Enabled);
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the ssl pair opened"),
    };
    assert_eq!(
        error,
        DialError::Other {
            message: "ssl argument is incompatible with a ws:// URI".to_owned(),
        }
    );
    let refused = timeout(Duration::from_millis(50), listener.accept()).await;
    let _elapsed = refused.expect_err("the dialer connected without a handshake");
}

#[tokio::test]
async fn dialer_reports_a_socks_proxy_as_the_import_error() {
    let environment =
        BTreeMap::from([("SOCKS_PROXY".to_owned(), "socks5h://127.0.0.1:9".to_owned())]);
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment).expect("policy");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(Arc::new(policy));
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(
            &dialer,
            &request("ws://example.test/realtime", &ssl, &headers, 1024),
        ),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the socks proxy opened"),
    };
    assert_eq!(
        error,
        DialError::Other {
            message: "python-socks is required to use a SOCKS proxy".to_owned(),
        }
    );
}

#[tokio::test]
async fn dialer_reports_a_https_proxy_as_invalid_proxy() {
    let environment = BTreeMap::from([(
        "HTTPS_PROXY".to_owned(),
        "https://proxy.local:3128".to_owned(),
    )]);
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment).expect("policy");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(Arc::new(policy));
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(
            &dialer,
            &request("ws://example.test/realtime", &ssl, &headers, 1024),
        ),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the https proxy opened"),
    };
    assert_eq!(
        error,
        DialError::Other {
            message: "websockets can't connect through a https proxy".to_owned(),
        }
    );
}

// SSL_VERIFY=false on a wss URL keeps TLS, as llm_http_handler.py builds a context with CERT_NONE there
#[tokio::test]
async fn dialer_keeps_tls_for_a_wss_url_when_verification_is_disabled() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = JoinSet::new();
    tasks.spawn(record_first_byte(listener, Arc::clone(&seen)));
    let url = format!("wss://{address}/realtime");
    let ssl = RealtimeSsl::Insecure;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let outcome = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline");
    let error = match outcome {
        Err(error) => error,
        Ok(_backend) => panic!("the dropped wss socket opened"),
    };
    assert!(
        matches!(error, DialError::Os { .. }),
        "the dropped TLS handshake failed with {error:?}"
    );
    let first = seen.lock().expect("seen").first().copied();
    assert_eq!(
        first,
        Some(0x16),
        "the first byte was not a TLS handshake record"
    );
    tasks.abort_all();
}

#[tokio::test]
async fn size_limited_source_closes_a_frame_one_byte_over_the_cap_with_1009() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let frame = Message::Text(Utf8Bytes::from("x".repeat(AT_LIMIT_FRAME)));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_frames(listener, vec![frame.clone(), frame]));
    let mut source = dialed_source(address, MAX_SIZE).await;
    // the client's own cap is one over the session's, so the frame the session rejects arrives intact first
    let delivered = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline")
        .expect("a frame at the client's cap arrives");
    assert_eq!(delivered, RealtimeMessage::Text("x".repeat(AT_LIMIT_FRAME)));
    let mut limited = SizeLimitedSource {
        inner: source,
        max_message_size_bytes: MAX_SIZE,
    };
    let outcome = timeout(TEST_TIMEOUT, limited.recv(None))
        .await
        .expect("deadline");
    let closed = expect_closed(
        outcome,
        "the oversized frame failed instead of closing",
        "the oversized frame arrived",
    );
    assert_oversized_close(&closed, MAX_SIZE);
    assert_eq!(backend_close_from(&closed).code, 1009);
    tasks.abort_all();
}

#[tokio::test]
async fn dialer_closes_a_frame_over_its_own_cap_with_1009() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let frame = Message::Text(Utf8Bytes::from("x".repeat(OVER_LIMIT_FRAME)));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_close_after(listener, Some(frame)));
    let mut source = dialed_source(address, MAX_SIZE).await;
    let outcome = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline");
    let closed = expect_closed(
        outcome,
        "the oversized frame failed instead of closing",
        "the oversized frame arrived",
    );
    assert_oversized_close(&closed, MAX_SIZE);
    let (code, reason) = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(code, 1009);
    assert_eq!(reason, "message too big");
}

// the dialer's socket pings every 20 s, so the shared client is opened with a short keepalive and wrapped in the dialer's source
async fn keepalive_source(address: SocketAddr) -> super::halves::DialedSource {
    let connection = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url: Url::parse(&format!("ws://{address}/realtime")).expect("url"),
            headers: &[],
            tls: None,
            max_message_size: 1024,
            open_timeout: TEST_TIMEOUT,
            keepalive: Some(Keepalive {
                interval: Duration::from_millis(30),
                timeout: Duration::from_millis(30),
            }),
            proxy: None,
        }),
    )
    .await
    .expect("deadline")
    .expect("connect");
    super::halves::DialedSource {
        receiver: connection.receiver,
        sender: connection.sender.clone(),
        max_size: 1024,
    }
}

#[tokio::test]
async fn source_reports_a_missed_pong_as_the_keepalive_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_silent(listener));
    let mut source = keepalive_source(address).await;
    let outcome = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline");
    let closed = match outcome {
        Err(RelayReceiveError::Closed(closed)) => closed,
        Err(_error) => panic!("the keepalive timeout failed instead of closing"),
        Ok(_message) => panic!("the silent server sent a frame"),
    };
    assert_eq!(closed.received, None);
    assert_eq!(
        closed.message,
        "sent 1011 (internal error) keepalive ping timeout; no close frame received"
    );
    let (code, reason) = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(code, 1011);
    assert_eq!(reason, "keepalive ping timeout");
}

#[tokio::test]
async fn dialer_reports_an_end_without_a_close_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_end(listener));
    let mut source = dialed_source(address, 1024).await;
    let outcome = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline");
    let closed = expect_closed(
        outcome,
        "the ended socket failed instead of closing",
        "the ended socket sent a frame",
    );
    assert_eq!(closed.received, None);
    assert_eq!(closed.message, "no close frame received or sent");
    tasks.abort_all();
}

#[tokio::test]
async fn source_applies_the_decode_flag_to_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let frames = vec![
        Message::Binary(Bytes::from_static(b"hello")),
        Message::Text(Utf8Bytes::from("world")),
        Message::Binary(Bytes::from_static(&[0xff, 0xfe])),
    ];
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_frames(listener, frames));
    let mut source = dialed_source(address, 1024).await;
    let decoded = timeout(TEST_TIMEOUT, source.recv(Some(true)))
        .await
        .expect("deadline")
        .expect("binary decoded");
    assert_eq!(decoded, RealtimeMessage::Text("hello".to_owned()));
    let raw = timeout(TEST_TIMEOUT, source.recv(Some(false)))
        .await
        .expect("deadline")
        .expect("text as bytes");
    assert_eq!(raw, RealtimeMessage::Binary(b"world".to_vec()));
    let outcome = timeout(TEST_TIMEOUT, source.recv(Some(true)))
        .await
        .expect("deadline");
    let failed = match outcome {
        Err(RelayReceiveError::Failed(failed)) => failed,
        Err(_error) => panic!("invalid UTF-8 closed the socket"),
        Ok(_message) => panic!("invalid UTF-8 decoded"),
    };
    assert_eq!(
        failed.message,
        "the realtime backend sent a binary frame that is not UTF-8"
    );
    tasks.abort_all();
}

#[tokio::test]
async fn dialer_sends_the_backend_headers_in_order() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_with_headers(listener, Arc::clone(&seen)));
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let mut headers = HeaderMap::new();
    drop(headers.insert("x-request-id".to_owned(), Json::Text("one".to_owned())));
    drop(headers.insert(
        "authorization".to_owned(),
        Json::Text("Bearer two".to_owned()),
    ));
    let dialer = web_dialer(no_proxy());
    let backend = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline")
    .expect("opened");
    drop(backend);
    timeout(TEST_TIMEOUT, tasks.join_next())
        .await
        .expect("server")
        .expect("server")
        .expect("server task");
    let recorded = seen.lock().expect("recorded headers");
    assert_eq!(
        custom_headers(recorded.as_slice()),
        [("authorization", "Bearer two"), ("x-request-id", "one")]
    );
}

#[tokio::test]
async fn sender_writes_text_binary_and_a_normal_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_sent_frames(listener));
    let mut backend = dialed_backend(address, 1024).await;
    backend
        .sender
        .send(RealtimeMessage::Text("hello".to_owned()))
        .await
        .expect("text");
    backend
        .sender
        .send(RealtimeMessage::Binary(b"raw".to_vec()))
        .await
        .expect("binary");
    backend.sender.close().await.expect("close");
    let (text, binary, close) = timeout(TEST_TIMEOUT, tasks.join_next())
        .await
        .expect("server")
        .expect("server")
        .expect("server task");
    assert_sent_frames(&text, &binary, &close);
}

#[tokio::test]
async fn probe_answers_ok_on_an_accepted_upgrade_and_closes_1000() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_close_after(listener, None));
    let dialer = web_dialer(no_proxy());
    let connection = probe_connection(format!("ws://{address}/realtime"));
    timeout(TEST_TIMEOUT, dialer.probe(&connection))
        .await
        .expect("deadline")
        .expect("probe");
    let (code, reason) = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(code, 1000);
    assert_eq!(reason, "");
}

#[tokio::test]
async fn probe_reports_a_rejected_upgrade_as_invalid_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_attempts(
        listener,
        Arc::clone(&attempts),
        Some(StatusCode::FORBIDDEN),
    ));
    let dialer = web_dialer(no_proxy());
    let connection = probe_connection(format!("ws://{address}/realtime"));
    let error = timeout(TEST_TIMEOUT, dialer.probe(&connection))
        .await
        .expect("deadline")
        .expect_err("the probe opened a rejected upgrade");
    assert_eq!(error.class, "InvalidStatus");
    assert_eq!(error.status, Some(403));
    assert_eq!(
        error.message,
        "server rejected WebSocket connection: HTTP 403"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    tasks.abort_all();
}

#[test]
fn dialer_timeouts_match_websockets() {
    assert_eq!(super::DEFAULT_KEEPALIVE.interval, Duration::from_secs(20));
    assert_eq!(super::DEFAULT_KEEPALIVE.timeout, Duration::from_secs(20));
    assert_eq!(super::PROBE_OPEN_TIMEOUT, Duration::from_secs(10));
}

#[rstest]
#[case::ok(1000, "", "1000 (OK)")]
#[case::ok_with_a_reason(1000, "bye", "1000 (OK) bye")]
#[case::going_away(1001, "", "1001 (going away)")]
#[case::protocol_error(1002, "", "1002 (protocol error)")]
#[case::unsupported_data(1003, "", "1003 (unsupported data)")]
#[case::no_status(1005, "", "1005 (no status received [internal])")]
#[case::abnormal_closure(1006, "", "1006 (abnormal closure [internal])")]
#[case::invalid_payload(1007, "", "1007 (invalid frame payload data)")]
#[case::policy_violation(1008, "", "1008 (policy violation)")]
#[case::message_too_big(1009, "too big", "1009 (message too big) too big")]
#[case::mandatory_extension(1010, "", "1010 (mandatory extension)")]
#[case::internal_error(
    1011,
    "keepalive ping timeout",
    "1011 (internal error) keepalive ping timeout"
)]
#[case::service_restart(1012, "", "1012 (service restart)")]
#[case::try_again_later(1013, "", "1013 (try again later)")]
#[case::bad_gateway(1014, "", "1014 (bad gateway)")]
#[case::tls_handshake(1015, "", "1015 (TLS handshake failure [internal])")]
#[case::registered_low(3000, "", "3000 (registered)")]
#[case::registered_high(3999, "", "3999 (registered)")]
#[case::private_use(4000, "bye", "4000 (private use) bye")]
#[case::private_use_high(4999, "", "4999 (private use)")]
#[case::unknown(7, "", "7 (unknown)")]
fn close_texts_match_websockets(#[case] code: u16, #[case] reason: &str, #[case] expected: &str) {
    assert_eq!(super::halves::close_text(code, reason), expected);
}
