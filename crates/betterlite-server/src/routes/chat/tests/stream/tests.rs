#![cfg(test)]

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::routes::chat::{ErrorLayer, chat_routes_with_call_id};
use crate::server::chat_setup::chat_executor;
use betterlite_error::normalize::Normalizer;
use betterlite_openai::chat::execution::ChatError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use std::fs::read_to_string;
use tempfile::NamedTempFile;
use tower::ServiceExt as _;
use tracing::subscriber::{set_default, with_default};

const FIRST: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"}}]}\n\n";
const SECOND: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"second\"}}]}\n\n";

async fn gateway(upstream: &TcpListener, tasks: &mut JoinSet<()>) -> (ProxiedClient, String) {
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 10}}}}\n"
    );
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    tasks.spawn(async move {
        axum::serve(
            listener,
            chat_routes_with_call_id(Arc::new(executor), false).unwrap(),
        )
        .await
        .unwrap();
    });
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    (ProxiedClient::outbound(&policy).unwrap(), url)
}

async fn accept_request(listener: TcpListener) -> TcpStream {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    assert!(head.contains("accept: text/event-stream"));
    assert!(head.contains("authorization: Bearer k"));
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    assert!(String::from_utf8(body).unwrap().contains("\"stream\":true"));
    socket
}

async fn paused_provider(
    listener: TcpListener,
    advance: oneshot::Receiver<()>,
    closed: oneshot::Sender<()>,
) {
    let mut socket = accept_request(listener).await;
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 10000\r\n\r\n",
        )
        .await
        .unwrap();
    socket.write_all(FIRST.as_bytes()).await.unwrap();
    advance.await.unwrap();
    socket.write_all(SECOND.as_bytes()).await.unwrap();
    let eof = timeout(Duration::from_secs(1), socket.read_u8())
        .await
        .unwrap();
    assert_eq!(eof.unwrap_err().kind(), ErrorKind::UnexpectedEof);
    closed.send(()).unwrap();
}
#[tokio::test]
async fn streams_before_next_chunk_and_cancels_on_disconnect() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut tasks = JoinSet::new();
    let (client, url) = gateway(&upstream, &mut tasks).await;
    let (advance, resumed) = oneshot::channel();
    let (closed, disconnected) = oneshot::channel();
    tasks.spawn(paused_provider(upstream, resumed, closed));
    let mut response = client
        .client
        .post(url)
        .header("content-type", "application/json")
        .body(r#"{"model":"g","messages":[],"stream":true}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        timeout(Duration::from_secs(1), response.chunk())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        FIRST
    );
    advance.send(()).unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), response.chunk())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        SECOND
    );
    drop(response);
    timeout(Duration::from_secs(1), disconnected)
        .await
        .unwrap()
        .unwrap();
    tasks.shutdown().await;
}

#[rstest]
#[case::timeout(ChatError::Timeout, 408, "408_UPSTREAM_TIMEOUT")]
#[case::authentication(ChatError::MissingApiKey, 401, "401_AUTHENTICATION_FAILED")]
fn error_normalization_tests_behavior(
    #[case] error: ChatError,
    #[case] status: u16,
    #[case] code: &str,
) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let layer = ErrorLayer {
        normalizer: Normalizer::new().unwrap(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    };
    with_default(subscriber, || {
        layer.log_normalized_error(&error, "private-message", status);
    });
    let output = read_to_string(log.path()).unwrap();
    assert!(output.contains(&format!("normalized_error=\"{code}\"")));
    assert!(!output.contains("private-message"));
}

#[tokio::test]
async fn an_unrouted_chat_failure_is_logged_before_dispatch() {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:1', api_key: k}}\n")
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let router = chat_routes_with_call_id(Arc::new(executor), false).unwrap();
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let request = Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"missing","messages":[]}"#))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    drop(guard);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let output = read_to_string(log.path()).unwrap();
    assert!(
        output.contains("chat request failed before dispatch"),
        "{output}"
    );
    assert!(output.contains("missing"), "{output}");
}
