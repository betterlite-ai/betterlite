#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::anthropic_http_stream::call::{OpenedStream, StreamCall, StreamClients, make_call};
use crate::anthropic_http_stream::error::StreamFailure;
use crate::stream_decode::DecodedChunk;
use crate::stream_decode::error::DecodeError;

use super::fixture::{
    DATA, FRAME, FixedCounter, FixedIds, messages, request_headers, stream_build,
};

const TRUNCATED_HEAD: &str =
    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 4096\r\n\r\n";

#[tokio::test]
async fn anthropic_http_stream_partial_body_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_truncated(listener));
    let client = proxied_client();
    let drain_client = client.clone();
    let mut opened = open(&api_base, &client).await;
    let first = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("the first frame arrives before the close")
        .expect("the frame is read")
        .expect("the frame decodes");
    assert!(matches!(first, DecodedChunk::Event(_)));
    let ended = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("a truncated body ends the stream instead of hanging")
        .expect("the truncation is reported");
    assert!(matches!(ended, Err(DecodeError::Other { .. })));
    drop(opened);
    drain_client
        .drain(Duration::from_secs(10))
        .await
        .expect("the transport owner drains after the truncation");
    timeout(Duration::from_secs(10), tasks.join_next())
        .await
        .expect("the local server ends")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn anthropic_http_stream_cancel_upstream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_base = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (closed, closed_rx) = oneshot::channel::<()>();
    let mut tasks = JoinSet::new();
    tasks.spawn(serve_until_closed(listener, closed));
    let client = proxied_client();
    let drain_client = client.clone();
    let mut opened = open(&api_base, &client).await;
    let first = timeout(Duration::from_secs(10), opened.completion_stream.next())
        .await
        .expect("the first frame arrives")
        .expect("the frame is read")
        .expect("the frame decodes");
    assert!(matches!(first, DecodedChunk::Event(_)));
    drop(opened);
    timeout(Duration::from_secs(10), closed_rx)
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

async fn open(api_base: &str, client: &ProxiedClient) -> OpenedStream<StreamFailure, FixedCounter> {
    let messages = messages();
    let reverse = IndexMap::new();
    let call = StreamCall {
        api_base,
        headers: request_headers(),
        data: DATA,
        messages: &messages,
        timeout: Duration::from_secs(10),
        json_mode: false,
        speed: None,
        tool_name_reverse_map: &reverse,
    };
    make_call(
        StreamClients {
            supplied: Some(client),
            module_level_aclient: client,
            module_level_client: client,
        },
        &call,
        stream_build(&FixedIds),
    )
    .await
    .expect("the local server answers 200")
}

async fn serve_truncated(listener: TcpListener) {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(TRUNCATED_HEAD.as_bytes()).await.unwrap();
    socket
        .write_all(format!("{FRAME}\n").as_bytes())
        .await
        .unwrap();
    drop(socket);
}

async fn serve_until_closed(listener: TcpListener, closed: oneshot::Sender<()>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    socket.write_all(TRUNCATED_HEAD.as_bytes()).await.unwrap();
    socket
        .write_all(format!("{FRAME}\n").as_bytes())
        .await
        .unwrap();
    let mut rest = Vec::new();
    socket.read_to_end(&mut rest).await.unwrap();
    let _sent = closed.send(());
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}
