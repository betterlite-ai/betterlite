use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt as _, Stream, StreamExt as _};
use reqwest::header::HeaderMap;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Error as WebSocketError;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{accept_async, accept_hdr_async};

use super::{CALLER, ws_guardrail};
use crate::cato::WsSsl;
use crate::cato::error::CatoError;
use crate::cato::inspection::hooks::streaming_iterator_hook;
use crate::cato::tests::support::{assert_header, chat_data};
use crate::cato::tests::{list, object, text};
use crate::cato::transport::stream::{CatoSocket, DONE_FRAME, MAX_FRAME_BYTES};
use crate::cato::transport::websocket::WebSocketConnector;

struct Report {
    headers: BTreeMap<String, String>,
    received: Vec<String>,
}

async fn read_frames<S>(socket: &mut S, limit: usize) -> Vec<String>
where
    S: Stream<Item = Result<Message, WebSocketError>> + Unpin,
{
    let mut texts = Vec::new();
    while texts.len() < limit {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => texts.push(text.as_str().to_owned()),
            Some(Ok(_)) => {}
            Some(Err(_)) | None => break,
        }
    }
    texts
}

// a Cato stream endpoint: it answers only once the client sent `wait_for` frames, then records the rest
async fn cato_stream_server(frames: Vec<Message>, wait_for: usize) -> (String, JoinSet<Report>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_cato_stream(listener, frames, wait_for));
    (format!("ws://{address}/fw/v1/analyze/stream"), tasks)
}

async fn serve_cato_stream(listener: TcpListener, frames: Vec<Message>, wait_for: usize) -> Report {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let mut headers = BTreeMap::new();
    let mut socket = accept_hdr_async(
        stream,
        |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            for (name, value) in request.headers() {
                if let Ok(value) = value.to_str() {
                    headers.insert(name.as_str().to_owned(), value.to_owned());
                }
            }
            Ok(response)
        },
    )
    .await
    .expect("handshake");
    let mut received = read_frames(&mut socket, wait_for).await;
    for frame in frames {
        socket.send(frame).await.expect("frame");
    }
    received.extend(read_frames(&mut socket, usize::MAX).await);
    Report { headers, received }
}

struct TlsServer {
    url: String,
    ca_pem: String,
    tasks: JoinSet<()>,
}

async fn tls_server() -> TlsServer {
    let certified =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
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
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let port = listener.local_addr().expect("address").port();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_tls_stream(listener, acceptor));
    TlsServer {
        url: format!("wss://127.0.0.1:{port}/fw/v1/analyze/stream"),
        ca_pem: certified.cert.pem(),
        tasks,
    }
}

async fn serve_tls_stream(listener: TcpListener, acceptor: TlsAcceptor) {
    let (stream, _peer) = listener.accept().await.expect("accept");
    let Ok(tls) = acceptor.accept(stream).await else {
        return;
    };
    let Ok(mut socket) = accept_async(tls).await else {
        return;
    };
    socket
        .send(Message::Text(Utf8Bytes::from(DONE_FRAME)))
        .await
        .expect("done frame");
    while socket.next().await.is_some() {}
}

async fn connect_websocket(
    url: &str,
    ssl: Option<WsSsl>,
    read: Duration,
) -> Result<CatoSocket, CatoError> {
    let guardrail = ws_guardrail(url, ssl, Duration::from_secs(5));
    WebSocketConnector::from(&guardrail)
        .connect(url, &HeaderMap::new(), Duration::from_secs(5), read)
        .await
}

#[tokio::test]
async fn websocket_connector_exchanges_text_and_binary_frames_with_a_local_server() {
    let frames = vec![
        Message::Ping(Vec::new().into()),
        Message::Text(Utf8Bytes::from(r#"{"verified_chunk": {"id": "one"}}"#)),
        Message::Binary(b"binary".to_vec().into()),
    ];
    let (url, mut tasks) = cato_stream_server(frames, 0).await;
    let mut socket = connect_websocket(&url, None, Duration::from_secs(5))
        .await
        .expect("connected");
    let first = socket
        .reader
        .recv_message()
        .await
        .expect("frame")
        .expect("text");
    assert_eq!(first, br#"{"verified_chunk": {"id": "one"}}"#.to_vec());
    let second = socket
        .reader
        .recv_message()
        .await
        .expect("frame")
        .expect("binary");
    assert_eq!(second, b"binary".to_vec());
    socket.writer.send_text("client frame").await.expect("sent");
    drop(socket);
    let report = tasks
        .join_next()
        .await
        .expect("server task")
        .expect("report");
    assert_eq!(report.received, vec!["client frame".to_owned()]);
}

#[tokio::test]
async fn websocket_connector_accepts_a_frame_of_the_limit_and_rejects_a_larger_one() {
    let frames = vec![
        Message::Text(Utf8Bytes::from("y".repeat(MAX_FRAME_BYTES))),
        Message::Text(Utf8Bytes::from("x".repeat(MAX_FRAME_BYTES + 1))),
    ];
    let (url, mut tasks) = cato_stream_server(frames, 0).await;
    let mut socket = connect_websocket(&url, None, Duration::from_secs(5))
        .await
        .expect("connected");
    let at_limit = socket
        .reader
        .recv_message()
        .await
        .expect("frame")
        .expect("text");
    assert_eq!(at_limit.len(), MAX_FRAME_BYTES);
    let error = socket.reader.recv_message().await.expect_err("too large");
    assert!(matches!(error, CatoError::WebSocket { .. }));
    tasks.abort_all();
}

#[tokio::test]
async fn websocket_connector_bounds_the_handshake_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let url = format!(
        "ws://{}/fw/v1/analyze/stream",
        listener.local_addr().expect("address")
    );
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (_stream, _peer) = listener.accept().await.expect("accept");
        std::future::pending::<()>().await;
    });
    let guardrail = ws_guardrail(&url, None, Duration::from_secs(5));
    let result = WebSocketConnector::from(&guardrail)
        .connect(
            &url,
            &HeaderMap::new(),
            Duration::from_millis(50),
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(CatoError::ConnectTimeout)));
    tasks.abort_all();
}

#[tokio::test]
async fn websocket_connector_bounds_the_frame_wait() {
    let (url, mut tasks) = cato_stream_server(Vec::new(), 0).await;
    let mut socket = connect_websocket(&url, None, Duration::from_millis(50))
        .await
        .expect("connected");
    let error = socket.reader.recv_message().await.expect_err("no frame");
    assert!(matches!(error, CatoError::StreamTimeout));
    tasks.abort_all();
}

#[tokio::test]
async fn streaming_iterator_hook_relays_the_stream_with_cato_headers() {
    let chunk = object(&[("choices", list(&[text("one")]))]);
    let verified = object(&[("verified_chunk", chunk.clone())]);
    let verified_frame = serde_json::to_string(&verified).expect("frame");
    let frames = vec![
        Message::Text(Utf8Bytes::from(verified_frame)),
        Message::Text(Utf8Bytes::from(DONE_FRAME)),
    ];
    let (url, mut tasks) = cato_stream_server(frames, 2).await;
    let guardrail = ws_guardrail(&url, None, Duration::from_secs(5));
    let (output, mut delivered) = mpsc::channel(8);
    let (sender, chunks) = mpsc::channel(8);
    sender.send(chunk.clone()).await.expect("chunk");
    drop(sender);
    streaming_iterator_hook(&guardrail, &chat_data(), CALLER, chunks, output)
        .await
        .expect("relayed");
    assert_eq!(delivered.recv().await, Some(chunk.clone()));
    assert!(delivered.recv().await.is_none());
    let joined = tasks.join_next().await.expect("server task");
    let report = joined.expect("report");
    assert_header(&report.headers, "authorization", "Bearer test-key");
    assert_header(&report.headers, "x-cato-litellm-hook", "output");
    assert_header(&report.headers, "x-cato-call-id", "call-1");
    assert_header(&report.headers, "x-cato-user-email", "user@example.test");
    assert_header(&report.headers, "x-cato-gateway-key-alias", "alias");
    assert_eq!(
        report.received,
        vec![
            serde_json::to_string(&chunk).expect("chunk"),
            DONE_FRAME.to_owned()
        ]
    );
}

#[tokio::test]
async fn websocket_connector_trusts_a_disabled_and_a_ca_file_verification() {
    let mut disabled = tls_server().await;
    let mut socket =
        connect_websocket(&disabled.url, Some(WsSsl::Disabled), Duration::from_secs(5))
            .await
            .expect("connected");
    let frame = socket
        .reader
        .recv_message()
        .await
        .expect("frame")
        .expect("text");
    assert_eq!(frame, DONE_FRAME.as_bytes().to_vec());

    let mut trusted = tls_server().await;
    let mut ca_file = tempfile::NamedTempFile::new().expect("ca file");
    ca_file
        .write_all(trusted.ca_pem.as_bytes())
        .expect("ca pem");
    let path = ca_file.path().to_str().expect("ca path").to_owned();
    let mut socket = connect_websocket(
        &trusted.url,
        Some(WsSsl::CaFile(path)),
        Duration::from_secs(5),
    )
    .await
    .expect("connected");
    let frame = socket
        .reader
        .recv_message()
        .await
        .expect("frame")
        .expect("text");
    assert_eq!(frame, DONE_FRAME.as_bytes().to_vec());
    disabled.tasks.abort_all();
    trusted.tasks.abort_all();
}

#[tokio::test]
async fn websocket_connector_rejects_a_self_signed_server_on_the_system_roots() {
    let native = tls_server().await;
    let result = connect_websocket(
        &native.url,
        Some(WsSsl::SystemRoots),
        Duration::from_secs(5),
    )
    .await;
    assert!(matches!(result, Err(CatoError::WebSocket { .. })));
    let default = tls_server().await;
    let result = connect_websocket(&default.url, None, Duration::from_secs(5)).await;
    assert!(matches!(result, Err(CatoError::WebSocket { .. })));
}

#[tokio::test]
async fn websocket_connector_reports_a_ca_file_it_cannot_read() {
    let directory = tempfile::tempdir().expect("temp dir");
    let missing = directory.path().join("missing-ca.pem");
    let path = missing.to_str().expect("ca path").to_owned();
    let url = "wss://127.0.0.1:1/fw/v1/analyze/stream";
    let result = connect_websocket(url, Some(WsSsl::CaFile(path)), Duration::from_secs(5)).await;
    assert!(matches!(result, Err(CatoError::WsCaFile { .. })));
}

#[tokio::test]
async fn websocket_connector_reports_a_ca_file_that_is_not_a_certificate() {
    let mut ca_file = tempfile::NamedTempFile::new().expect("ca file");
    ca_file
        .write_all(b"not a certificate")
        .expect("ca contents");
    let path = ca_file.path().to_str().expect("ca path").to_owned();
    let url = "wss://127.0.0.1:1/fw/v1/analyze/stream";
    let result = connect_websocket(url, Some(WsSsl::CaFile(path)), Duration::from_secs(5)).await;
    assert!(matches!(result, Err(CatoError::Tls { .. })));
}
