use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::text_completion_execution::CompletionExecutor;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_request::CompletionRequest;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::{OnceCell, oneshot};
use tokio::task::JoinSet;
use tokio::time::timeout;

const FIRST: &str = "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":7,\"model\":\"up\",\"choices\":[{\"text\":\"first\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n";
const SECOND: &str = "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":7,\"model\":\"up\",\"choices\":[{\"text\":\"second\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n";

fn deployment(api_base: &str) -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("text-model"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(String::from("openai/gpt-3.5-turbo-instruct")),
            api_base: Some(api_base.to_owned()),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy policy"),
    );
    ProxiedClient::outbound(&policy).expect("client")
}

// The provider sends one event, waits for the client to consume it, then sends the next and holds
// the connection until the client closes it.
async fn paused_provider(
    listener: TcpListener,
    first_delivered: oneshot::Receiver<()>,
    closed: oneshot::Sender<()>,
) {
    let (mut socket, _) = listener.accept().await.expect("accepted");
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.expect("request head"));
    }
    let head = String::from_utf8(received).expect("request head text");
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .expect("content length")
        .parse()
        .expect("numeric length");
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.expect("request body");
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 10000\r\n\r\n",
        )
        .await
        .expect("response head");
    socket
        .write_all(FIRST.as_bytes())
        .await
        .expect("first event");
    first_delivered.await.expect("first event consumed");
    socket
        .write_all(SECOND.as_bytes())
        .await
        .expect("second event");
    let eof = timeout(Duration::from_secs(5), socket.read_u8())
        .await
        .expect("the client closed the upstream socket");
    assert_eq!(eof.expect_err("eof").kind(), ErrorKind::UnexpectedEof);
    closed.send(()).expect("closed signal");
}

#[tokio::test]
async fn text_streaming_text_stream_tests_behavior() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = upstream.local_addr().expect("address");
    let mut tasks = JoinSet::new();
    let (advance, first_delivered) = oneshot::channel();
    let (closed, disconnected) = oneshot::channel();
    tasks.spawn(paused_provider(upstream, first_delivered, closed));

    let deployment = deployment(&format!("http://{address}/v1"));
    let executor = CompletionExecutor {
        client: client(),
        catalog: OnceCell::new(),
    };
    let request =
        CompletionRequest::parse(br#"{"model":"text-model","prompt":"hello","stream":true}"#)
            .expect("completion request");
    let redactor = Redactor::new(0).expect("redactor");
    let mut logger = TracingCompletionLog;
    let answer = executor
        .stream(&deployment, &request, &redactor, &mut logger)
        .await
        .expect("stream");
    let mut stream = answer.stream;

    let first = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("the first chunk reaches the client while the provider waits")
        .expect("a chunk")
        .expect("shaped chunk");
    assert_eq!(first.choices[0].delta.content.as_deref(), Some("first"));
    advance
        .send(())
        .expect("let the provider send the next event");
    let second = timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("second chunk")
        .expect("a chunk")
        .expect("shaped chunk");
    assert_eq!(second.choices[0].delta.content.as_deref(), Some("second"));

    drop(stream);
    timeout(Duration::from_secs(1), disconnected)
        .await
        .expect("the upstream closes within one second of the disconnect")
        .expect("closed signal");
    tasks
        .join_next()
        .await
        .expect("provider task")
        .expect("provider");
}
