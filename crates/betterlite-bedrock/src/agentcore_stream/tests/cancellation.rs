use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::agentcore_stream::adapter;
use crate::agentcore_stream::tests::support::{FixedIds, FixtureLogger};
use crate::agentcore_stream::transport::{HandlerKind, StreamHandler};
use crate::agentcore_stream::{StreamDeps, StreamRequest};

#[tokio::test]
async fn agentcore_stream_releases_transport_on_drop() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/invoke", listener.local_addr().unwrap());
    let (closed, closed_rx) = oneshot::channel::<Vec<u8>>();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_one_frame(listener, closed));
    let client = proxied_client();
    assert_eq!(client.kind(), HandlerKind::Async);
    let drain_client = client.clone();
    let transport: Arc<dyn StreamHandler> = Arc::new(client);
    let logger = FixtureLogger::default();
    let data = Json::Object(IndexMap::default());
    let messages = Json::List(Vec::new());
    let request = StreamRequest {
        model: "agentcore-model",
        api_base: &api_base,
        headers: HeaderMap::new(),
        data: &data,
        signed_json_body: None,
        messages: &messages,
    };
    let deps = StreamDeps {
        supplied: None,
        default: transport,
        ids: Arc::new(FixedIds::new(&[])),
        logger: &logger,
    };
    let opening = adapter::open(&request, &deps, HandlerKind::Async);
    let mut stream = timeout(Duration::from_secs(10), opening)
        .await
        .unwrap()
        .unwrap();
    let first = timeout(Duration::from_secs(10), stream.next_chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.choices[0].delta.content.as_deref(), Some("one"));
    drop(stream);
    assert_released(closed_rx, drain_client, &mut tasks).await;
}

async fn serve_one_frame(listener: TcpListener, closed: oneshot::Sender<Vec<u8>>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 4096\r\n\r\n";
    socket.write_all(head.as_bytes()).await.unwrap();
    let frame = b"data: {\"event\":{\"contentBlockDelta\":{\"delta\":{\"text\":\"one\"}}}}\n";
    socket.write_all(frame).await.unwrap();
    let mut rest = Vec::new();
    socket.read_to_end(&mut rest).await.unwrap();
    drop(closed.send(rest));
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

async fn assert_released(
    closed_rx: oneshot::Receiver<Vec<u8>>,
    drain_client: ProxiedClient,
    tasks: &mut JoinSet<()>,
) {
    let _rest = timeout(Duration::from_secs(10), closed_rx)
        .await
        .expect("the dropped stream releases the connection")
        .unwrap();
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains after the drop");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the upstream server ends after the drop")
        .unwrap()
        .unwrap();
}
