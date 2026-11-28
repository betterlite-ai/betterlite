#![cfg(test)]

use std::future::pending;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::proxy::{ProxyCredentials, ProxyServer};
use betterlite_config::tls::Verification;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use http::StatusCode;
use http::header::{HeaderName, HeaderValue};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, ServerConfig};
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{WebSocketStream, accept_async, accept_hdr_async};
use url::Url;

use super::connect;
use super::connection::{PeerClose, WebSocketConnection, WebSocketReceive, WebSocketReceiver};
use super::error::WebSocketClientError;
use super::request::{Keepalive, WebSocketRequest};
use crate::tls::websocket_client_config;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN_TIMEOUT: Duration = Duration::from_millis(100);
const MESSAGE_LIMIT: usize = 16;
const OVERSIZED_MESSAGE: usize = 17;
const PING_INTERVAL: Duration = Duration::from_millis(50);
const PING_TIMEOUT: Duration = Duration::from_millis(50);
const SERVER_SILENCE: Duration = Duration::from_millis(250);
const PONG_INTERVAL: Duration = Duration::from_millis(300);
const PONG_HALF_INTERVAL: Duration = Duration::from_millis(150);
const PONG_TIMEOUT: Duration = Duration::from_millis(300);
const PONG_WINDOW: Duration = Duration::from_millis(700);

// the recorder keeps what the client asked with, and answers with a rejection when the test asks for one
struct Recorder<'headers> {
    headers: &'headers mut Vec<(String, String)>,
    target: &'headers mut String,
    rejection: Option<StatusCode>,
}

impl Callback for Recorder<'_> {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        *self.target = request.uri().to_string();
        for (name, value) in request.headers() {
            if let Ok(text) = value.to_str() {
                self.headers
                    .push((name.as_str().to_owned(), text.to_owned()));
            }
        }
        match self.rejection {
            Some(status) => {
                let mut rejected = ErrorResponse::new(Some("forbidden".to_owned()));
                *rejected.status_mut() = status;
                Err(rejected)
            }
            None => Ok(response),
        }
    }
}

// the server side every connect test runs: one text in, then a text, a binary and a peer close out
async fn answer_client<S>(socket: &mut WebSocketStream<S>) -> String
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let read = socket.next().await.expect("client frame").expect("frame");
    socket
        .send(Message::Text(Utf8Bytes::from("world")))
        .await
        .expect("text");
    socket
        .send(Message::Binary(Bytes::copy_from_slice(b"\x00\x01")))
        .await
        .expect("binary");
    let close = CloseFrame {
        code: CloseCode::Library(4000),
        reason: Utf8Bytes::from("bye"),
    };
    socket
        .send(Message::Close(Some(close)))
        .await
        .expect("close");
    match read {
        Message::Text(text) => text.as_str().to_owned(),
        other @ (Message::Binary(_)
        | Message::Ping(_)
        | Message::Pong(_)
        | Message::Close(_)
        | Message::Frame(_)) => panic!("client frame: {other:?}"),
    }
}

async fn serve_plain(listener: TcpListener) -> (String, Vec<(String, String)>) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut headers = Vec::new();
    let mut target = String::new();
    let mut socket = accept_hdr_async(
        stream,
        Recorder {
            headers: &mut headers,
            target: &mut target,
            rejection: None,
        },
    )
    .await
    .expect("handshake");
    assert_eq!(answer_client(&mut socket).await, "hello");
    (target, headers)
}

async fn serve_tls(listener: TcpListener, config: Arc<ServerConfig>) -> String {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let tls = TlsAcceptor::from(config)
        .accept(stream)
        .await
        .expect("tls accept");
    let mut socket = accept_async(tls).await.expect("handshake");
    answer_client(&mut socket).await
}

async fn serve_rejection(listener: TcpListener) -> bool {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut headers = Vec::new();
    let mut target = String::new();
    let recorder = Recorder {
        headers: &mut headers,
        target: &mut target,
        rejection: Some(StatusCode::FORBIDDEN),
    };
    accept_hdr_async(stream, recorder).await.is_err()
}

async fn serve_hung_upgrade(listener: TcpListener) {
    let (mut stream, _peer) = listener.accept().await.expect("accept");
    let mut head = [0_u8; 1024];
    let read = stream.read(&mut head).await.expect("read");
    assert!(read > 0);
    pending::<()>().await;
}

async fn serve_oversized_message(listener: TcpListener) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    let text = Utf8Bytes::from("x".repeat(OVERSIZED_MESSAGE));
    socket
        .send(Message::Text(text))
        .await
        .expect("oversized text");
    pending::<()>().await;
}

async fn serve_silent(listener: TcpListener) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    sleep(SERVER_SILENCE).await;
    let ping = socket.next().await.expect("ping").expect("frame");
    assert!(matches!(ping, Message::Ping(_)));
    let frame = socket.next().await.expect("close").expect("frame");
    let Message::Close(close) = frame else {
        panic!("expected a close frame");
    };
    let code = close.expect("close code");
    (u16::from(code.code), code.reason.as_str().to_owned())
}

// a server that records the one binary frame a test sends, so send_binary has an observer
async fn serve_binary_frame(listener: TcpListener) -> Vec<u8> {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    let frame = socket.next().await.expect("frame").expect("frame");
    match frame {
        Message::Binary(bytes) => bytes.to_vec(),
        other @ (Message::Text(_)
        | Message::Ping(_)
        | Message::Pong(_)
        | Message::Close(_)
        | Message::Frame(_)) => panic!("frame: {other:?}"),
    }
}

// a server that reads the one close frame a test sends
async fn serve_close_frame(listener: TcpListener) -> (u16, String) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut socket = accept_async(stream).await.expect("handshake");
    let frame = socket.next().await.expect("close").expect("frame");
    let Message::Close(Some(close)) = frame else {
        panic!("expected a close frame");
    };
    (u16::from(close.code), close.reason.as_str().to_owned())
}

// a server that answers every ping the way tungstenite does, recording when each one arrived
async fn serve_ponging(listener: TcpListener, pings: Arc<Mutex<Vec<Duration>>>) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let started = Instant::now();
    let mut socket = accept_async(stream).await.expect("handshake");
    while let Some(frame) = socket.next().await {
        if matches!(frame.expect("frame"), Message::Ping(_)) {
            pings.lock().expect("pings").push(started.elapsed());
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut buffer = [0_u8; 512];
    while !head
        .windows(4)
        .any(|window| window == b"\r\n\r\n".as_slice())
    {
        let read = stream.read(&mut buffer).await.expect("read");
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(head).expect("head text")
}

async fn serve_proxy(listener: TcpListener, forward: Option<SocketAddr>) -> String {
    let (mut downstream, _peer) = listener.accept().await.expect("accept");
    let head = read_head(&mut downstream).await;
    match forward {
        None => {
            downstream
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n",
                )
                .await
                .expect("response");
        }
        Some(target) => {
            let mut upstream = TcpStream::connect(target).await.expect("upstream");
            downstream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .expect("response");
            drop(copy_bidirectional(&mut downstream, &mut upstream).await);
        }
    }
    head
}

async fn tls_server() -> TlsServer {
    let certified =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
    let provider = Arc::new(aws_lc_rs::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certified.cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                certified.signing_key.serialize_der(),
            )),
        )
        .expect("server config");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    TlsServer {
        listener,
        config: Arc::new(config),
        ca_pem: certified.cert.pem(),
    }
}

struct TlsServer {
    listener: TcpListener,
    config: Arc<ServerConfig>,
    ca_pem: String,
}

// a client opened over a plain ws URL with the given headers, through the proxy when one is given
async fn plain_connection(
    url: &str,
    headers: &[(HeaderName, HeaderValue)],
    proxy: Option<&ProxyServer>,
) -> WebSocketConnection {
    let request = WebSocketRequest {
        url: Url::parse(url).expect("url"),
        headers,
        tls: None,
        max_message_size: 1024,
        open_timeout: TEST_TIMEOUT,
        keepalive: None,
        proxy,
    };
    timeout(TEST_TIMEOUT, connect(request))
        .await
        .expect("deadline")
        .expect("connect")
}

async fn tls_connection(url: Url, tls: Arc<ClientConfig>) -> WebSocketConnection {
    let request = WebSocketRequest {
        url,
        headers: &[],
        tls: Some(tls),
        max_message_size: 1024,
        open_timeout: TEST_TIMEOUT,
        keepalive: None,
        proxy: None,
    };
    timeout(TEST_TIMEOUT, connect(request))
        .await
        .expect("deadline")
        .expect("connect")
}

async fn assert_text_then_binary(receiver: &mut WebSocketReceiver) {
    assert_eq!(
        receiver.recv().await.expect("recv"),
        WebSocketReceive::Text("world".to_owned())
    );
    assert_eq!(
        receiver.recv().await.expect("recv"),
        WebSocketReceive::Binary(Bytes::from_static(b"\x00\x01"))
    );
}

async fn assert_peer_close(receiver: &mut WebSocketReceiver, code: u16, reason: &str) {
    assert_eq!(
        receiver.recv().await.expect("recv"),
        WebSocketReceive::Close(PeerClose {
            code,
            reason: reason.to_owned(),
        })
    );
}

fn assert_handshake_headers(recorded: &[(String, String)], address: SocketAddr) {
    let names: Vec<&str> = recorded
        .iter()
        .map(|(name, _value)| name.as_str())
        .collect();
    assert_eq!(
        &names[..5],
        [
            "host",
            "connection",
            "upgrade",
            "sec-websocket-version",
            "sec-websocket-key",
        ]
    );
    // removing its own five headers reorders the caller's two in the HeaderMap, so only their set is fixed
    let mut custom: Vec<&str> = names[5..].to_vec();
    custom.sort_unstable();
    assert_eq!(custom, ["authorization", "x-request-id"]);
    let host = recorded
        .iter()
        .find(|(name, _value)| name.as_str() == "host")
        .map(|(_name, value)| value.as_str());
    let expected_host = address.to_string();
    assert_eq!(host, Some(expected_host.as_str()));
    let request_id = recorded
        .iter()
        .find(|(name, _value)| name.as_str() == "x-request-id")
        .map(|(_name, value)| value.as_str());
    assert_eq!(request_id, Some("one"));
    let authorization = recorded
        .iter()
        .find(|(name, _value)| name.as_str() == "authorization")
        .map(|(_name, value)| value.as_str());
    assert_eq!(authorization, Some("Bearer two"));
}

#[tokio::test]
async fn websocket_client_tests_open_plain_socket_with_custom_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_plain(listener));
    let headers = [
        (
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_static("one"),
        ),
        (
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer two"),
        ),
    ];
    let url = format!("ws://{address}/realtime?model=x");
    let mut connection = plain_connection(&url, &headers, None).await;
    connection.sender.send_text("hello").await.expect("send");
    assert_text_then_binary(&mut connection.receiver).await;
    assert_peer_close(&mut connection.receiver, 4000, "bye").await;
    let (target, recorded) = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(target, "/realtime?model=x");
    assert_handshake_headers(&recorded, address);
}

#[tokio::test]
async fn websocket_client_tests_send_a_binary_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_binary_frame(listener));
    let url = format!("ws://{address}/realtime");
    let connection = plain_connection(&url, &[], None).await;
    connection.sender.send_binary(b"raw").await.expect("send");
    let payload = timeout(TEST_TIMEOUT, tasks.join_next())
        .await
        .expect("deadline")
        .expect("server")
        .expect("server task");
    assert_eq!(payload, b"raw".to_vec());
}

#[tokio::test]
async fn websocket_client_tests_send_a_normal_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_close_frame(listener));
    let url = format!("ws://{address}/realtime");
    let connection = plain_connection(&url, &[], None).await;
    connection.sender.close(1000, "bye").await.expect("close");
    let (code, reason) = timeout(TEST_TIMEOUT, tasks.join_next())
        .await
        .expect("deadline")
        .expect("server")
        .expect("server task");
    assert_eq!(code, 1000);
    assert_eq!(reason, "bye");
}

#[tokio::test]
async fn websocket_client_tests_exchange_frames_over_tls() {
    let server = tls_server().await;
    let address = server.listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_tls(server.listener, server.config));
    let mut ca_file = NamedTempFile::new().expect("ca file");
    ca_file.write_all(server.ca_pem.as_bytes()).expect("ca pem");
    let ca_path = ca_file.path().to_str().expect("ca path").to_owned();
    let config = websocket_client_config(&Verification::CaFile(ca_path)).expect("tls config");
    let url = Url::parse(&format!("wss://{address}/realtime")).expect("url");
    let mut connection = tls_connection(url, Arc::new(config)).await;
    connection.sender.send_text("hello").await.expect("send");
    assert_text_then_binary(&mut connection.receiver).await;
    assert_eq!(
        tasks
            .join_next()
            .await
            .expect("server")
            .expect("server task"),
        "hello"
    );
}

#[tokio::test]
async fn websocket_client_tests_report_the_upgrade_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_rejection(listener));
    let url = Url::parse(&format!("ws://{address}/realtime")).expect("url");
    let outcome = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url,
            headers: &[],
            tls: None,
            max_message_size: 1024,
            open_timeout: TEST_TIMEOUT,
            keepalive: None,
            proxy: None,
        }),
    )
    .await
    .expect("deadline");
    assert!(matches!(
        outcome,
        Err(WebSocketClientError::Status { status: 403 })
    ));
    let rejected = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert!(rejected);
}

#[tokio::test]
async fn websocket_client_tests_time_out_a_hung_upgrade() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_hung_upgrade(listener));
    let url = Url::parse(&format!("ws://{address}/realtime")).expect("url");
    let started = Instant::now();
    let outcome = connect(WebSocketRequest {
        url,
        headers: &[],
        tls: None,
        max_message_size: 1024,
        open_timeout: OPEN_TIMEOUT,
        keepalive: None,
        proxy: None,
    })
    .await;
    assert!(matches!(outcome, Err(WebSocketClientError::Timeout)));
    assert!(started.elapsed() >= OPEN_TIMEOUT);
    tasks.abort_all();
}

#[tokio::test]
async fn websocket_client_tests_enforce_the_message_cap() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_oversized_message(listener));
    let url = Url::parse(&format!("ws://{address}/realtime")).expect("url");
    let mut connection = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url,
            headers: &[],
            tls: None,
            max_message_size: MESSAGE_LIMIT,
            open_timeout: TEST_TIMEOUT,
            keepalive: None,
            proxy: None,
        }),
    )
    .await
    .expect("deadline")
    .expect("connect");
    let outcome = timeout(TEST_TIMEOUT, connection.receiver.recv())
        .await
        .expect("deadline");
    assert!(matches!(
        outcome,
        Err(WebSocketClientError::Capacity {
            size: OVERSIZED_MESSAGE,
            max_size: MESSAGE_LIMIT
        })
    ));
    tasks.abort_all();
}

#[tokio::test]
async fn websocket_client_tests_fail_the_socket_when_a_pong_misses() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_silent(listener));
    let url = Url::parse(&format!("ws://{address}/realtime")).expect("url");
    let mut connection = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url,
            headers: &[],
            tls: None,
            max_message_size: 1024,
            open_timeout: TEST_TIMEOUT,
            keepalive: Some(Keepalive {
                interval: PING_INTERVAL,
                timeout: PING_TIMEOUT,
            }),
            proxy: None,
        }),
    )
    .await
    .expect("deadline")
    .expect("connect");
    let outcome = timeout(TEST_TIMEOUT, connection.receiver.recv())
        .await
        .expect("deadline");
    assert!(matches!(
        outcome,
        Err(WebSocketClientError::KeepaliveTimeout)
    ));
    let (code, reason) = tasks
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(code, 1011);
    assert_eq!(reason, "keepalive ping timeout");
}

#[tokio::test]
async fn websocket_client_tests_answer_pongs_without_closing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let pings = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_ponging(listener, Arc::clone(&pings)));
    let url = Url::parse(&format!("ws://{address}/realtime")).expect("url");
    let mut connection = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url,
            headers: &[],
            tls: None,
            max_message_size: 1024,
            open_timeout: TEST_TIMEOUT,
            keepalive: Some(Keepalive {
                interval: PONG_INTERVAL,
                timeout: PONG_TIMEOUT,
            }),
            proxy: None,
        }),
    )
    .await
    .expect("deadline")
    .expect("connect");
    let outcome = timeout(PONG_WINDOW, connection.receiver.recv()).await;
    assert!(
        matches!(outcome, Err(_elapsed)),
        "the pongs kept recv waiting"
    );
    let pinged = pings.lock().expect("pings");
    let first = pinged.first().copied().expect("a ping");
    let count = pinged.len();
    drop(pinged);
    assert!(
        first >= PONG_HALF_INTERVAL,
        "the first ping came after {first:?}"
    );
    assert!(count <= 20, "the client sent {count} pings");
    tasks.abort_all();
}

#[tokio::test]
async fn websocket_client_tests_report_a_proxy_status_and_send_basic_auth() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_proxy(listener, None));
    let proxy = ProxyServer {
        host: "127.0.0.1".to_owned(),
        port: address.port(),
        credentials: Some(ProxyCredentials {
            username: "user".to_owned(),
            password: SecretString::from("secret"),
        }),
    };
    let url = Url::parse("ws://example.test/realtime").expect("url");
    let outcome = timeout(
        TEST_TIMEOUT,
        connect(WebSocketRequest {
            url,
            headers: &[],
            tls: None,
            max_message_size: 1024,
            open_timeout: TEST_TIMEOUT,
            keepalive: None,
            proxy: Some(&proxy),
        }),
    )
    .await
    .expect("deadline");
    assert!(matches!(
        outcome,
        Err(WebSocketClientError::ProxyStatus { status: 407 })
    ));
    let head = tasks.join_next().await.expect("proxy").expect("proxy task");
    assert!(head.starts_with("CONNECT example.test:80 HTTP/1.1\r\n"));
    assert!(head.contains("host: example.test\r\n"));
    let expected = STANDARD.encode("user:secret");
    assert!(head.contains(&format!("proxy-authorization: Basic {expected}\r\n")));
}

#[tokio::test]
async fn websocket_client_tests_tunnel_through_a_connect_proxy() {
    let backend = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let backend_address = backend.local_addr().expect("backend address");
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.expect("proxy");
    let proxy_address = proxy_listener.local_addr().expect("proxy address");
    let mut servers = JoinSet::new();
    servers.spawn(serve_plain(backend));
    let mut proxies = JoinSet::new();
    proxies.spawn(serve_proxy(proxy_listener, Some(backend_address)));
    let proxy = ProxyServer {
        host: "127.0.0.1".to_owned(),
        port: proxy_address.port(),
        credentials: None,
    };
    let url = format!("ws://{backend_address}/realtime");
    let mut connection = plain_connection(&url, &[], Some(&proxy)).await;
    connection.sender.send_text("hello").await.expect("send");
    assert_text_then_binary(&mut connection.receiver).await;
    assert_peer_close(&mut connection.receiver, 4000, "bye").await;
    drop(connection);
    let head = proxies
        .join_next()
        .await
        .expect("proxy")
        .expect("proxy task");
    assert!(head.starts_with(&format!("CONNECT {backend_address} HTTP/1.1\r\n")));
    let (target, _recorded) = servers
        .join_next()
        .await
        .expect("server")
        .expect("server task");
    assert_eq!(target, "/realtime");
}
