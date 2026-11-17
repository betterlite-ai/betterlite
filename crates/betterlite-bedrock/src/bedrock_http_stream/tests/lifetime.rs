use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::support::{Call, Consume, DATA, FIRST, Outcome, invoke, messages, open as open_stream};
use crate::agentcore_stream::transport::{HandlerKind, PostRequest, StreamHandler, StreamHead};
use crate::bedrock_http_stream::stream::StreamFailure;

const HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/vnd.amazon.eventstream\r\n";
const WAIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn bedrock_http_stream_body_lifetime() {
    let messages = messages();
    let frame = hex::decode(FIRST).unwrap();
    eof_phase(&frame, &messages).await;
    dropped_phase(&frame, &messages).await;
    short_body_phase(&frame, &messages).await;
}

// EOF ends the stream after the tally ran, and the dropped response releases the connection.
async fn eof_phase(frame: &[u8], messages: &Json) {
    let mut body = format!("{:x}\r\n", frame.len()).into_bytes();
    body.extend_from_slice(frame);
    body.extend_from_slice(b"\r\n0\r\n\r\n");
    let mut phase = Phase::start(
        format!("{HEAD}transfer-encoding: chunked\r\n\r\n"),
        body,
        false,
    )
    .await;
    let outcome = phase.run(messages, Consume::All).await;
    assert!(outcome.error.is_none());
    assert!(outcome.failures.is_empty());
    assert_eq!(outcome.chunks.len(), 1);
    assert_eq!(
        outcome
            .headers
            .as_ref()
            .unwrap()
            .get("content-type")
            .unwrap(),
        "application/vnd.amazon.eventstream"
    );
    phase.release().await;
    assert_eq!(phase.posts.load(Ordering::Relaxed), 1);
}

// A consumer that stops before the body ends drops the response mid-frame, and the server sees the close.
async fn dropped_phase(frame: &[u8], messages: &Json) {
    let mut partial_body = frame.to_vec();
    partial_body.extend_from_slice(&frame[..20]);
    let mut phase = Phase::start(
        format!("{HEAD}content-length: 4096\r\n\r\n"),
        partial_body,
        false,
    )
    .await;
    let outcome = phase.run(messages, Consume::First).await;
    assert!(outcome.error.is_none());
    assert_eq!(outcome.chunks.len(), 1);
    phase.release().await;
    assert_eq!(phase.posts.load(Ordering::Relaxed), 1);
}

// A body that ends early is a read failure, and the stream neither retries it nor reports a frame.
async fn short_body_phase(frame: &[u8], messages: &Json) {
    let mut phase = Phase::start(
        format!("{HEAD}content-length: 4096\r\n\r\n"),
        frame[..20].to_vec(),
        true,
    )
    .await;
    let outcome = phase.run(messages, Consume::All).await;
    assert!(outcome.error.is_none());
    assert!(outcome.chunks.is_empty());
    assert_eq!(outcome.failures.len(), 1);
    assert!(matches!(&outcome.failures[0], StreamFailure::Read(_)));
    assert_eq!(outcome.failure_statuses(), vec![None]);
    phase.release().await;
    assert_eq!(phase.posts.load(Ordering::Relaxed), 1);
}

struct Phase {
    api_base: String,
    client: ProxiedClient,
    posts: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
    closed: oneshot::Receiver<()>,
}

impl Phase {
    async fn start(head: String, body: Vec<u8>, hang_up: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_base = format!("http://{}/invoke", listener.local_addr().unwrap());
        let (signal, closed) = oneshot::channel();
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, head, body, hang_up, signal));
        Self {
            api_base,
            client: proxied_client(),
            posts: Arc::new(AtomicUsize::new(0)),
            tasks,
            closed,
        }
    }

    async fn run(&self, messages: &Json, consume: Consume) -> Outcome {
        let call = invoke(&self.api_base, DATA, messages);
        let handler: Arc<dyn StreamHandler> = Arc::new(Recording {
            client: self.client.clone(),
            posts: Arc::clone(&self.posts),
        });
        timeout(WAIT, open_stream(Call::Invoke(call), handler, consume))
            .await
            .expect("the stream call finishes")
    }

    async fn release(&mut self) {
        self.client
            .drain(WAIT)
            .await
            .expect("the transport owner drains after the drop");
        timeout(WAIT, &mut self.closed)
            .await
            .expect("the dropped stream closes its connection")
            .unwrap();
        timeout(WAIT, self.tasks.join_next())
            .await
            .expect("the server ends after the drop")
            .unwrap()
            .unwrap();
    }
}

struct Recording {
    client: ProxiedClient,
    posts: Arc<AtomicUsize>,
}

#[async_trait]
impl StreamHandler for Recording {
    fn kind(&self) -> HandlerKind {
        HandlerKind::Async
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.posts.fetch_add(1, Ordering::Relaxed);
        StreamHandler::post(&self.client, request).await
    }
}

async fn serve(
    listener: TcpListener,
    head: String,
    body: Vec<u8>,
    hang_up: bool,
    closed: oneshot::Sender<()>,
) {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    if hang_up {
        socket.shutdown().await.unwrap();
    }
    let mut rest = Vec::new();
    socket.read_to_end(&mut rest).await.unwrap();
    drop(closed.send(()));
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}
