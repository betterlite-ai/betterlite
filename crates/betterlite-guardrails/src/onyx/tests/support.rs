use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::onyx::OnyxGuardrail;
use betterlite_http::proxy::base_builder;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

pub(crate) const API_KEY: &str = "oracle-key";

#[derive(Clone, Default)]
pub(crate) struct CapturedLog(pub(crate) Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned log buffer"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct MockOnyx {
    pub(crate) base: String,
    pub(crate) seen: Arc<Mutex<Vec<Seen>>>,
    pub(crate) accepted: Arc<AtomicUsize>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock_onyx(status: StatusCode, reply: String) -> MockOnyx {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_by_handler = Arc::clone(&accepted);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&seen_by_handler);
        let counter = Arc::clone(&accepted_by_handler);
        let reply_for_response = reply.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            recorder.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                content_type: headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
                body: bytes.to_vec(),
            });
            (
                status,
                [("content-type", "application/json")],
                reply_for_response,
            )
        }
    };
    let (base, server) = serve(Router::new().fallback(answer)).await;
    MockOnyx {
        base,
        seen,
        accepted,
        _server: server,
    }
}

// The reply is a denial so a missing read timeout turns the test into a Blocked result, not a slow pass.
pub(crate) async fn silent_onyx() -> MockOnyx {
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_by_handler = Arc::clone(&accepted);
    let answer = move |_uri: Uri, _bytes: Bytes| {
        let counter = Arc::clone(&accepted_by_handler);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_secs(30)).await;
            (StatusCode::OK, String::from(r#"{"allowed": false}"#))
        }
    };
    let (base, server) = serve(Router::new().fallback(answer)).await;
    MockOnyx {
        base,
        seen: Arc::new(Mutex::new(Vec::new())),
        accepted,
        _server: server,
    }
}

pub(crate) enum Script {
    Delayed {
        delay: Duration,
        reply: String,
    },
    Split {
        head: Duration,
        tail: Duration,
        reply: String,
    },
    IncompleteThenDeny {
        reply: String,
    },
    TruncatedChunkedThenDeny {
        reply: String,
    },
    IllegalChunkThenDeny {
        reply: String,
    },
    TruncatedServerErrorThenDeny {
        reply: String,
    },
    ResetThenDeny {
        reply: String,
    },
}

pub(crate) struct RawMock {
    pub(crate) base: String,
    pub(crate) accepted: Arc<AtomicUsize>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn scripted_mock(script: Script) -> RawMock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    let mut server = JoinSet::new();
    server.spawn(run_script(listener, count, script));
    RawMock {
        base,
        accepted,
        _server: server,
    }
}

async fn run_script(
    listener: TcpListener,
    accepted: Arc<AtomicUsize>,
    script: Script,
) -> io::Result<()> {
    match script {
        Script::Delayed { delay, reply } => {
            delayed_reply(&listener, &accepted, delay, &reply).await
        }
        Script::Split { head, tail, reply } => {
            split_reply(&listener, &accepted, head, tail, &reply).await
        }
        Script::IncompleteThenDeny { reply } => {
            misbehave_then_deny(&listener, &accepted, &reply, FirstFailure::Incomplete).await
        }
        Script::TruncatedChunkedThenDeny { reply } => {
            misbehave_then_deny(&listener, &accepted, &reply, FirstFailure::TruncatedChunked).await
        }
        Script::IllegalChunkThenDeny { reply } => {
            misbehave_then_deny(&listener, &accepted, &reply, FirstFailure::IllegalChunk).await
        }
        Script::TruncatedServerErrorThenDeny { reply } => {
            misbehave_then_deny(
                &listener,
                &accepted,
                &reply,
                FirstFailure::TruncatedServerError,
            )
            .await
        }
        Script::ResetThenDeny { reply } => {
            misbehave_then_deny(&listener, &accepted, &reply, FirstFailure::Reset).await
        }
    }
}

async fn delayed_reply(
    listener: &TcpListener,
    accepted: &AtomicUsize,
    delay: Duration,
    reply: &str,
) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    sleep(delay).await;
    write_response(&mut stream, reply).await
}

async fn split_reply(
    listener: &TcpListener,
    accepted: &AtomicUsize,
    head: Duration,
    tail: Duration,
    reply: &str,
) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    sleep(head).await;
    write_head(&mut stream, 200, reply.len()).await?;
    sleep(tail).await;
    stream.write_all(reply.as_bytes()).await?;
    stream.shutdown().await
}

enum FirstFailure {
    Incomplete,
    TruncatedChunked,
    IllegalChunk,
    TruncatedServerError,
    Reset,
}

async fn misbehave_then_deny(
    listener: &TcpListener,
    accepted: &AtomicUsize,
    reply: &str,
    first: FirstFailure,
) -> io::Result<()> {
    match first {
        FirstFailure::Incomplete => incomplete_body(listener, accepted).await?,
        FirstFailure::TruncatedChunked => truncated_chunked_body(listener, accepted).await?,
        FirstFailure::IllegalChunk => illegal_chunk_header(listener, accepted).await?,
        FirstFailure::TruncatedServerError => truncated_server_error(listener, accepted).await?,
        FirstFailure::Reset => reset_connection(listener, accepted).await?,
    }
    let mut second = accept_one(listener, accepted).await?;
    write_response(&mut second, reply).await
}

async fn incomplete_body(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    write_head(&mut stream, 200, 100).await?;
    stream.write_all(b"{\"all").await?;
    stream.shutdown().await
}

async fn truncated_chunked_body(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    write_chunked_head(&mut stream).await?;
    stream.write_all(b"10\r\n{\"allowed\":").await?;
    stream.shutdown().await
}

async fn illegal_chunk_header(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    write_chunked_head(&mut stream).await?;
    stream.write_all(b"ZZZ\r\n").await?;
    stream.shutdown().await
}

async fn truncated_server_error(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<()> {
    let mut stream = accept_one(listener, accepted).await?;
    write_head(&mut stream, 500, 100).await?;
    stream.write_all(b"{\"all").await?;
    stream.shutdown().await
}

async fn reset_connection(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<()> {
    let (mut stream, _) = listener.accept().await?;
    accepted.fetch_add(1, Ordering::SeqCst);
    let mut byte = [0_u8; 1];
    let _read = stream.read(&mut byte).await;
    // unread request bytes at close make the kernel send a reset instead of a fin
    sleep(Duration::from_millis(20)).await;
    Ok(())
}

async fn accept_one(listener: &TcpListener, accepted: &AtomicUsize) -> io::Result<TcpStream> {
    let (mut stream, _) = listener.accept().await?;
    accepted.fetch_add(1, Ordering::SeqCst);
    let mut buffer = [0_u8; 4096];
    let _read = stream.read(&mut buffer).await;
    Ok(stream)
}

async fn write_head(stream: &mut TcpStream, status: u16, body_length: usize) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {body_length}\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await
}

async fn write_chunked_head(stream: &mut TcpStream) -> io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
        )
        .await
}

async fn write_response(stream: &mut TcpStream, reply: &str) -> io::Result<()> {
    write_head(stream, 200, reply.len()).await?;
    stream.write_all(reply.as_bytes()).await?;
    stream.shutdown().await
}

async fn serve(router: Router) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

pub(crate) fn guardrail(base: &str) -> OnyxGuardrail {
    timed_guardrail(base, 5.0)
}

pub(crate) fn timed_guardrail(base: &str, seconds: f64) -> OnyxGuardrail {
    let environment = BTreeMap::from([(String::from("ONYX_API_KEY"), String::from(API_KEY))]);
    OnyxGuardrail::new(
        None,
        Some(base.to_owned()),
        Some(seconds),
        &environment,
        base_builder(),
    )
    .unwrap()
}

pub(crate) fn object(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn inputs(json: &str) -> BTreeMap<String, GuardrailValue> {
    object(json)
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedRequest {
    pub(crate) payload: GuardrailValue,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) conversation_id: String,
}

pub(crate) fn captured(mock: &MockOnyx) -> CapturedRequest {
    let seen = mock.seen.lock().unwrap();
    serde_json::from_slice(&seen.first().unwrap().body).unwrap()
}

pub(crate) fn captured_path(mock: &MockOnyx) -> String {
    let seen = mock.seen.lock().unwrap();
    seen.first().unwrap().path.clone()
}

pub(crate) fn captured_content_type(mock: &MockOnyx) -> Option<String> {
    let seen = mock.seen.lock().unwrap();
    let content_type = seen.first().and_then(|call| call.content_type.clone());
    drop(seen);
    content_type
}
