#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::websocket_client::request::{Keepalive, WebSocketRequest};
use betterlite_openai::openai_base_config::Headers;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use http::StatusCode;
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
    DialRequest, RealtimeSsl, SizeLimitedSource, open_realtime_backend_ws,
};
use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_connection_plan::{ProbeConnection, RealtimeProbeDialer};
use crate::api::realtime_contracts::close::{CloseFrame, backend_close_from};
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

struct Rejector {
    status: StatusCode,
}

impl Callback for Rejector {
    fn on_request(
        self,
        _request: &Request,
        _response: Response,
    ) -> Result<Response, ErrorResponse> {
        let rejection = Response::builder()
            .status(self.status)
            .body(Some("rejected".to_owned()))
            .expect("rejection");
        Err(rejection)
    }
}

fn web_dialer(proxy: Arc<ProxyPolicy>) -> WebSocketDialer {
    WebSocketDialer { proxy }
}

fn no_proxy() -> Arc<ProxyPolicy> {
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy"))
}

fn request<'request>(
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

fn probe_connection(url: String) -> ProbeConnection {
    ProbeConnection {
        url,
        headers: Headers::new(),
        ssl: RealtimeSsl::Disabled,
        max_size: 1024,
    }
}

// a proxy that answers every upgrade attempt with the same status, counting the attempts
async fn serve_rejection(listener: TcpListener, attempts: Arc<AtomicUsize>, status: StatusCode) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        attempts.fetch_add(1, Ordering::SeqCst);
        drop(tokio_tungstenite::accept_hdr_async(stream, Rejector { status }).await);
    }
}

// a server that accepts and never answers, holding each attempt open until the open timeout fires
async fn serve_hung(listener: TcpListener, attempts: Arc<AtomicUsize>) {
    let mut held = Vec::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        attempts.fetch_add(1, Ordering::SeqCst);
        held.push(stream);
        assert!(held.len() <= ATTEMPTS);
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

async fn serve_frame_and_close(listener: TcpListener, frame: Message) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    socket.send(frame).await.expect("frame");
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

async fn serve_probe(listener: TcpListener) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    let read = socket.next().await.expect("close").expect("frame");
    let Message::Close(Some(close)) = read else {
        panic!("expected a close frame");
    };
    (u16::from(close.code), close.reason.as_str().to_owned())
}

#[tokio::test]
async fn dialer_answers_a_rejected_upgrade_with_the_status_after_one_attempt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_rejection(
        listener,
        Arc::clone(&attempts),
        StatusCode::UNAUTHORIZED,
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
    tasks.spawn(serve_hung(listener, Arc::clone(&attempts)));
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
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
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
async fn size_limited_source_closes_a_frame_one_byte_over_the_cap_with_1009() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let frame = Message::Text(Utf8Bytes::from("x".repeat(AT_LIMIT_FRAME)));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_frames(listener, vec![frame.clone(), frame]));
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let backend = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, MAX_SIZE)),
    )
    .await
    .expect("deadline")
    .expect("opened");
    let mut source = backend.source.expect("source");
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
    let closed = match outcome {
        Err(RelayReceiveError::Closed(closed)) => closed,
        Err(_error) => panic!("the oversized frame failed instead of closing"),
        Ok(_message) => panic!("the oversized frame arrived"),
    };
    assert_eq!(
        closed.received,
        Some(CloseFrame {
            code: 1009,
            reason: "message too big".to_owned(),
        })
    );
    assert_eq!(
        closed.message,
        "the realtime backend message exceeds the 8-byte limit"
    );
    assert_eq!(backend_close_from(&closed).code, 1009);
    tasks.abort_all();
}

#[tokio::test]
async fn dialer_closes_a_frame_over_its_own_cap_with_1009() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let frame = Message::Text(Utf8Bytes::from("x".repeat(OVER_LIMIT_FRAME)));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_frame_and_close(listener, frame));
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let backend = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, MAX_SIZE)),
    )
    .await
    .expect("deadline")
    .expect("opened");
    let mut source = backend.source.expect("source");
    let outcome = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline");
    let closed = match outcome {
        Err(RelayReceiveError::Closed(closed)) => closed,
        Err(_error) => panic!("the oversized frame failed instead of closing"),
        Ok(_message) => panic!("the oversized frame arrived"),
    };
    assert_eq!(
        closed.received,
        Some(CloseFrame {
            code: 1009,
            reason: "message too big".to_owned(),
        })
    );
    assert_eq!(
        closed.message,
        "the realtime backend message exceeds the 8-byte limit"
    );
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
        betterlite_http::websocket_client::connect(WebSocketRequest {
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
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let backend = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline")
    .expect("opened");
    let mut source = backend.source.expect("source");
    let outcome = timeout(TEST_TIMEOUT, source.recv(None))
        .await
        .expect("deadline");
    let closed = match outcome {
        Err(RelayReceiveError::Closed(closed)) => closed,
        Err(_error) => panic!("the ended socket failed instead of closing"),
        Ok(_message) => panic!("the ended socket sent a frame"),
    };
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
    let url = format!("ws://{address}/realtime");
    let ssl = RealtimeSsl::Disabled;
    let headers = HeaderMap::new();
    let dialer = web_dialer(no_proxy());
    let backend = timeout(
        TEST_TIMEOUT,
        open_realtime_backend_ws(&dialer, &request(&url, &ssl, &headers, 1024)),
    )
    .await
    .expect("deadline")
    .expect("opened");
    let mut source = backend.source.expect("source");
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
async fn probe_answers_ok_on_an_accepted_upgrade_and_closes_1000() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_probe(listener));
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
    tasks.spawn(serve_rejection(
        listener,
        Arc::clone(&attempts),
        StatusCode::FORBIDDEN,
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

#[test]
fn close_texts_match_websockets() {
    assert_eq!(super::halves::close_text(1000, ""), "1000 (OK)");
    assert_eq!(super::halves::close_text(1000, "bye"), "1000 (OK) bye");
    assert_eq!(
        super::halves::close_text(1005, ""),
        "1005 (no status received [internal])"
    );
    assert_eq!(
        super::halves::close_text(1009, "too big"),
        "1009 (message too big) too big"
    );
    assert_eq!(
        super::halves::close_text(1011, "keepalive ping timeout"),
        "1011 (internal error) keepalive ping timeout"
    );
    assert_eq!(super::halves::close_text(3000, ""), "3000 (registered)");
    assert_eq!(
        super::halves::close_text(4000, "bye"),
        "4000 (private use) bye"
    );
    assert_eq!(super::halves::close_text(7, ""), "7 (unknown)");
}
