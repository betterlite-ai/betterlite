#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::{StreamExt as _, stream};
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::proxy::ProxiedClient;
use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::tests::{default_client, kind, reply, serve};

#[derive(Clone, Copy)]
enum Scenario {
    IncrementalBody,
    DrainWaitsForAnOpenStream,
    CancellationReleasesCapacity,
    DrainedClientRefusesNewWork,
    IdleConnectionsAreCapped,
}

fn get(url: &Url) -> OutboundRequest {
    OutboundRequest {
        method: Method::GET,
        url: url.clone(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    }
}

const BURST: usize = 40;

async fn burst(client: &ProxiedClient, url: &Url) {
    let answers: Vec<_> = stream::iter(0..BURST)
        .map(|_| client.send_buffered(get(url)))
        .buffer_unordered(BURST)
        .collect()
        .await;
    assert!(answers.iter().all(Result::is_ok));
}

// The second chunk is only written after the client has read the first, so a client that buffers the body hangs here.
async fn incremental(client: &ProxiedClient) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let go = Arc::new(Notify::new());
    let signal = Arc::clone(&go);
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        let first = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n1\r\na\r\n";
        stream.write_all(first).await.unwrap();
        signal.notified().await;
        stream.write_all(b"1\r\nb\r\n0\r\n\r\n").await.unwrap();
    });
    let mut body = client.send(get(&url)).await.unwrap().body_stream();
    let mut seen = Vec::new();
    while let Ok(Some(chunk)) = timeout(Duration::from_secs(2), body.next()).await {
        seen.push(String::from_utf8(chunk.unwrap()).unwrap());
        go.notify_one();
    }
    seen.join(" ")
}

#[rstest]
#[case::the_body_arrives_chunk_by_chunk(Scenario::IncrementalBody, "a b")]
#[case::drain_waits_for_an_open_stream_until_it_is_dropped(
    Scenario::DrainWaitsForAnOpenStream,
    "DrainTimeout drained"
)]
#[case::cancelling_a_request_releases_its_capacity(
    Scenario::CancellationReleasesCapacity,
    "cancelled drained"
)]
#[case::a_drained_client_refuses_new_work(Scenario::DrainedClientRefusesNewWork, "drained Closed")]
#[case::idle_connections_are_capped_per_host(Scenario::IdleConnectionsAreCapped, "48 connections")]
#[tokio::test]
async fn http_pool_tests_behavior(#[case] scenario: Scenario, #[case] expected: &str) {
    let client = default_client();
    let slow = serve(
        reply(&["200 OK", "content-length: 3"], b"abc"),
        Duration::ZERO,
    )
    .await;
    let observed = match scenario {
        Scenario::IncrementalBody => incremental(&client).await,
        Scenario::DrainWaitsForAnOpenStream => {
            let open = client.send(get(&slow.base)).await.unwrap();
            let waiting = client.drain(Duration::from_millis(100)).await.unwrap_err();
            drop(open);
            client.drain(Duration::from_secs(2)).await.unwrap();
            format!("{} drained", kind(&waiting))
        }
        Scenario::CancellationReleasesCapacity => {
            let never = serve(
                reply(&["200 OK", "content-length: 3"], b"abc"),
                Duration::from_secs(60),
            )
            .await;
            let started = timeout(Duration::from_millis(100), client.send(get(&never.base))).await;
            assert!(started.is_err());
            client.drain(Duration::from_secs(2)).await.unwrap();
            "cancelled drained".to_owned()
        }
        Scenario::DrainedClientRefusesNewWork => {
            client.drain(Duration::from_secs(1)).await.unwrap();
            let refused = client.send(get(&slow.base)).await.err().unwrap();
            format!("drained {}", kind(&refused))
        }
        Scenario::IdleConnectionsAreCapped => {
            let counted = serve(
                reply(&["200 OK", "content-length: 3"], b"abc"),
                Duration::from_millis(300),
            )
            .await;
            burst(&client, &counted.base).await;
            burst(&client, &counted.base).await;
            format!("{} connections", counted.connections.load(Ordering::SeqCst))
        }
    };
    assert_eq!(observed, expected);
}
