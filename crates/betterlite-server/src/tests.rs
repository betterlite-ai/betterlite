#![cfg(test)]
use crate::auth::parse_key_header;
use crate::server::chat_setup::chat_executor;
use crate::server::run;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use futures_util::future::join_all;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::time::Duration;
use std::{fs, path::Path};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
#[tokio::test]
async fn run_serves_requests_with_the_chat_executor_it_was_given() {
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let environment = BTreeMap::new();
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader
        .load("model_list:\n  - {model_name: g, litellm_params: {model: azure/m}}\n")
        .unwrap();
    let chat = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    let (served, response) = tokio::join!(
        timeout(Duration::from_secs(3), run(config, chat, "127.0.0.1", port)),
        timeout(Duration::from_secs(3), post_chat_once_listening(port))
    );
    assert!(
        served.is_err(),
        "run stopped while it should serve: {served:?}"
    );
    let answer = response.unwrap();
    assert!(answer.starts_with("HTTP/1.1 501"), "{answer}");
}
async fn post_chat_once_listening(port: u16) -> String {
    let mut connected = TcpStream::connect(("127.0.0.1", port)).await;
    while connected.is_err() {
        sleep(Duration::from_millis(10)).await;
        connected = TcpStream::connect(("127.0.0.1", port)).await;
    }
    let mut stream = connected.unwrap();
    let body = r#"{"model":"g","messages":[]}"#;
    let request = format!(
        "POST /chat/completions HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}
#[rstest]
#[case::repeated_bearer("Bearer test-Bearer master", "test-master")]
#[case::repeated_basic("Basic test-Basic master", "test-master")]
#[case::repeated_lowercase("bearer test-bearer master", "test-master")]
#[case::aws_empty_credential_before_master(
    "AWS4-HMAC-SHA256 Credential=/x, Credential=master/20261024/eu/service/aws4_request",
    "master"
)]
#[case::aws_first_credential(
    "AWS4-HMAC-SHA256 Credential=first, Credential=Bearer master/20261024/eu/service/aws4_request",
    "master"
)]
#[case::aws_first_bearer(
    "AWS4-HMAC-SHA256 Credential=Bearer test-master/d/r/s/aws4_request, Credential=other/d",
    "test-master"
)]
#[case::aws_bearer_before_plain(
    "AWS4-HMAC-SHA256 Credential=other/d, Credential=Bearer test-master/d",
    "test-master"
)]
#[case::aws_first_plain(
    "AWS4-HMAC-SHA256 Credential=test-master/d, Credential=other/d",
    "test-master"
)]
#[case::aws_two_bearers(
    "AWS4-HMAC-SHA256 Credential=Bearer test-master/d, Credential=Bearer other/d",
    "test-master"
)]
#[case::aws_empty("AWS4-HMAC-SHA256 Credential=/x", "")]
fn authorization_prefixes_match_litellm(#[case] value: &str, #[case] expected: &str) {
    let key = parse_key_header("authorization", value.as_bytes()).unwrap();
    assert_eq!(key.expose_secret(), expected);
}
#[rstest]
#[case::invalid_byte(b"invalid-\xFF")]
#[case::utf8_bytes(b"invalid-\xC3\xBF")]
fn non_utf8_header_is_used_as_wrong_key(#[case] value: &[u8]) {
    let key = parse_key_header("x-litellm-api-key", value).unwrap();
    assert_ne!(key.expose_secret(), "test-master");
}
#[rstest]
#[case::authorization("authorization")]
#[case::api_key("api-key")]
#[case::x_api_key("x-api-key")]
#[case::x_goog_api_key("x-goog-api-key")]
#[case::ocp_apim_subscription_key("ocp-apim-subscription-key")]
#[case::x_litellm_api_key("x-litellm-api-key")]
fn replays_every_key_header_fuzz_corpus_file(#[case] name: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/key_header");
    for entry in fs::read_dir(corpus).unwrap() {
        let input = fs::read(entry.unwrap().path()).unwrap();
        let _key = parse_key_header(name, &input);
    }
}

const CHAT_ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"up","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const EMBEDDING_ANSWER: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"up","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
const FIRST_FRAME: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"}}]}\n\n";
const SECOND_FRAME: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"second\"}}]}\n\n";
const CHAT_BODY: &str = r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#;
const STREAM_BODY: &str = r#"{"model":"g","messages":[],"stream":true}"#;

fn pipeline_config(base: &str) -> Config {
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', api_key: k, timeout: 10}}}}\n"
    );
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&source)
    .unwrap()
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn boot(config: Config, port: u16, tasks: &mut JoinSet<()>) {
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(7)).unwrap();
    tasks.spawn(async move {
        let serving = Box::pin(run(config, chat, "127.0.0.1", port));
        let _served = timeout(Duration::from_secs(10), serving).await;
    });
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    head
}

async fn serve_json(listener: TcpListener, count: usize) {
    for _request in 0..count {
        let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(3), listener.accept()).await
        else {
            return;
        };
        let head = read_request(&mut stream).await;
        let answer = if head.starts_with("POST /v1/embeddings") {
            EMBEDDING_ANSWER
        } else {
            CHAT_ANSWER
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        let _written = stream.write_all(response.as_bytes()).await;
        let _closed = stream.shutdown().await;
    }
}

async fn connect(port: u16) -> TcpStream {
    let mut connected = TcpStream::connect(("127.0.0.1", port)).await;
    while connected.is_err() {
        sleep(Duration::from_millis(10)).await;
        connected = TcpStream::connect(("127.0.0.1", port)).await;
    }
    connected.unwrap()
}

async fn post(port: u16, path: &str, body: &str) -> String {
    let mut stream = connect(port).await;
    let request = format!(
        "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

async fn read_until(stream: &mut TcpStream, received: &mut Vec<u8>, needle: &str) {
    while !received
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
    {
        let mut buffer = [0_u8; 4096];
        let read = timeout(Duration::from_secs(3), stream.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert!(read > 0, "the stream closed before {needle}");
        received.extend_from_slice(&buffer[..read]);
    }
}

async fn paused_upstream(
    listener: TcpListener,
    advance: oneshot::Receiver<()>,
    closed: oneshot::Sender<()>,
) {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(3), listener.accept()).await else {
        return;
    };
    let _head = read_request(&mut stream).await;
    let _head_written = stream
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 10000\r\n\r\n",
        )
        .await;
    let _first = stream.write_all(FIRST_FRAME.as_bytes()).await;
    advance.await.unwrap();
    let _second = stream.write_all(SECOND_FRAME.as_bytes()).await;
    let eof = timeout(Duration::from_secs(3), stream.read_u8())
        .await
        .unwrap();
    assert_eq!(eof.unwrap_err().kind(), ErrorKind::UnexpectedEof);
    closed.send(()).unwrap();
}

async fn failing_upstream(listener: TcpListener) {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(3), listener.accept()).await else {
        return;
    };
    let _head = read_request(&mut stream).await;
    let body = r#"{"error":{"message":"upstream exploded","type":"server_error","code":"500"}}"#;
    let response = format!(
        "HTTP/1.1 500 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _written = stream.write_all(response.as_bytes()).await;
    let _closed = stream.shutdown().await;
}

#[rstest]
#[case::chat_completion("/v1/chat/completions", CHAT_BODY, "\"content\":\"hi\"")]
#[case::embedding(
    "/v1/embeddings",
    r#"{"model":"g","input":"hello"}"#,
    "\"embedding\":[0.1,0.2]"
)]
#[tokio::test]
async fn call_dispatch_tests_behavior(
    #[case] path: &str,
    #[case] body: &str,
    #[case] expected: &str,
) {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let port = free_port().await;
    let mut tasks = JoinSet::new();
    boot(pipeline_config(&base), port, &mut tasks);
    tasks.spawn(serve_json(upstream, 1));
    let answer = post(port, path, body).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains(expected), "{answer}");
    tasks.shutdown().await;
}

#[tokio::test]
async fn real_server_stream_cancellation_tests_behavior() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let port = free_port().await;
    let mut tasks = JoinSet::new();
    boot(pipeline_config(&base), port, &mut tasks);
    let (advance, resumed) = oneshot::channel();
    let (closed, disconnected) = oneshot::channel();
    tasks.spawn(paused_upstream(upstream, resumed, closed));
    let mut client = connect(port).await;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{STREAM_BODY}",
        STREAM_BODY.len()
    );
    client.write_all(request.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    read_until(&mut client, &mut received, FIRST_FRAME).await;
    advance.send(()).unwrap();
    read_until(&mut client, &mut received, SECOND_FRAME).await;
    drop(client);
    timeout(Duration::from_secs(3), disconnected)
        .await
        .unwrap()
        .unwrap();
    tasks.shutdown().await;
}

#[tokio::test]
async fn real_server_streamed_failure_tests_behavior() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let port = free_port().await;
    let mut tasks = JoinSet::new();
    boot(pipeline_config(&base), port, &mut tasks);
    tasks.spawn(failing_upstream(upstream));
    let answer = post(port, "/v1/chat/completions", STREAM_BODY).await;
    assert!(answer.starts_with("HTTP/1.1 500"), "{answer}");
    assert!(answer.contains("\"code\":\"500\""), "{answer}");
    tasks.shutdown().await;
}

#[tokio::test]
async fn real_server_batch_tests_behavior() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", upstream.local_addr().unwrap());
    let port = free_port().await;
    let mut tasks = JoinSet::new();
    boot(pipeline_config(&base), port, &mut tasks);
    tasks.spawn(serve_json(upstream, 4));
    let posts = [
        post(port, "/v1/chat/completions", CHAT_BODY),
        post(port, "/v1/chat/completions", CHAT_BODY),
        post(port, "/v1/chat/completions", CHAT_BODY),
        post(port, "/v1/chat/completions", CHAT_BODY),
    ];
    let answers = join_all(posts).await;
    assert_eq!(answers.len(), 4);
    assert!(
        answers
            .iter()
            .all(|answer| answer.starts_with("HTTP/1.1 200"))
    );
    assert!(
        answers
            .iter()
            .all(|answer| answer.contains("\"content\":\"hi\""))
    );
    tasks.shutdown().await;
}
