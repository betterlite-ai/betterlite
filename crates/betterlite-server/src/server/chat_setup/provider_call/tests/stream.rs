use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::routes::chat::chat_routes_with_call_id;
use crate::server::chat_setup::chat_executor;

const FIRST: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"}}]}\n\n";
const SECOND: &str = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":7,\"model\":\"g\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"second\"}}]}\n\n";

async fn paused_provider(
    listener: TcpListener,
    advance: oneshot::Receiver<()>,
    closed: oneshot::Sender<()>,
) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
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

#[tokio::test]
async fn provider_framework_tests_behavior() {
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
    let first = timeout(Duration::from_secs(1), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first, FIRST);
    advance.send(()).unwrap();
    let second = timeout(Duration::from_secs(1), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(second, SECOND);
    drop(response);
    timeout(Duration::from_secs(1), disconnected)
        .await
        .unwrap()
        .unwrap();
    tasks.shutdown().await;
}
