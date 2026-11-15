use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

use super::fixture;
use crate::aim::error::AimError;
use crate::aim::settings::AimHook;
use crate::aim::stream::{AimSocketConnector as _, MAX_STREAM_FRAME_BYTES, StreamFrame};
use crate::aim::transport::AimWebSocketConnector;
use crate::aim::wire::AnalyzeResponse;

struct FrameServer {
    url: String,
    seen: Arc<Mutex<Option<String>>>,
    tasks: JoinSet<Vec<String>>,
}

struct Handshake {
    recorded: Arc<Mutex<Option<String>>>,
}

impl Callback for Handshake {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|header| header.to_str().ok())
            .map(str::to_owned);
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) = authorization;
        Ok(response)
    }
}

async fn frame_server(frames: Vec<String>) -> FrameServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/fw/v1/analyze/stream",
        listener.local_addr().unwrap()
    );
    let seen = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&seen);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (stream, _peer) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(stream, Handshake { recorded })
            .await
            .unwrap();
        for frame in frames {
            socket
                .send(Message::Text(Utf8Bytes::from(frame)))
                .await
                .unwrap();
        }
        let mut received = Vec::new();
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                received.push(text.as_str().to_owned());
            }
        }
        received
    });
    FrameServer { url, seen, tasks }
}

fn headers() -> HeaderMap {
    fixture(StatusCode::OK, "{}", &[])
        .guardrail
        .build_headers(AimHook::Output, Some("alias"), None, Some("call-1"))
        .unwrap()
}

#[tokio::test]
async fn aim_guardrail_websocket_connector_exchanges_text_frames_with_a_local_server() {
    let mut server = frame_server(vec![
        r#"{"verified_chunk": {"choices": [{"delta": {"content": "he"}}]}}"#.to_owned(),
        r#"{"done": true}"#.to_owned(),
    ])
    .await;
    let connector = AimWebSocketConnector;
    let mut socket = connector
        .connect(
            &server.url,
            &headers(),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    socket
        .writer
        .send_text(r#"{"choices":[]}"#.to_owned())
        .await
        .unwrap();
    socket
        .writer
        .send_text(r#"{"done":true}"#.to_owned())
        .await
        .unwrap();
    let first = socket.reader.recv_text().await.unwrap().unwrap();
    let second = socket.reader.recv_text().await.unwrap().unwrap();
    assert!(first.contains("verified_chunk"));
    assert_eq!(second, r#"{"done": true}"#);
    drop(socket);
    assert_eq!(
        server.seen.lock().unwrap().as_deref(),
        Some("Bearer aim-key")
    );
    let received = server.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(received, vec![r#"{"choices":[]}"#, r#"{"done":true}"#]);
}

#[tokio::test]
async fn aim_guardrail_websocket_connector_rejects_a_frame_larger_than_the_limit() {
    let oversized = "x".repeat(MAX_STREAM_FRAME_BYTES.saturating_add(1));
    let server = frame_server(vec![oversized]).await;
    let connector = AimWebSocketConnector;
    let mut socket = connector
        .connect(
            &server.url,
            &headers(),
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    let error = socket.reader.recv_text().await.unwrap_err();
    assert!(matches!(error, AimError::WebSocket(_)));
}

#[tokio::test]
async fn aim_guardrail_websocket_connector_bounds_the_handshake_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/fw/v1/analyze/stream",
        listener.local_addr().unwrap()
    );
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (_stream, _peer) = listener.accept().await.unwrap();
        time::sleep(Duration::from_secs(5)).await;
    });
    let result = AimWebSocketConnector
        .connect(
            &url,
            &headers(),
            Duration::from_millis(50),
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(result, Err(AimError::ConnectTimeout)));
    tasks.abort_all();
}

#[tokio::test]
async fn aim_guardrail_websocket_connector_bounds_the_frame_wait() {
    let server = frame_server(Vec::new()).await;
    let mut socket = AimWebSocketConnector
        .connect(
            &server.url,
            &headers(),
            Duration::from_secs(5),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
    let error = socket.reader.recv_text().await.unwrap_err();
    assert!(matches!(error, AimError::StreamTimeout));
}

#[test]
fn replays_the_aim_wire_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/aim_wire");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        serde_json::from_slice::<AnalyzeResponse>(&bytes).is_ok()
            && serde_json::from_slice::<StreamFrame>(&bytes).is_ok()
    }));
}
