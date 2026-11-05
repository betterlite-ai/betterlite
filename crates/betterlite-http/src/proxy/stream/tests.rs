#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;

use futures_util::future::ready;
use futures_util::{StreamExt as _, stream};
use h2::server::handshake;
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::error::HttpError;
use crate::proxy::limits::TransportLimits;
use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::tests::{default_client, kind, reply, serve};
use crate::proxy::{ProxiedClient, base_builder};

#[derive(Clone, Copy)]
enum Scenario {
    IncrementalBody,
    DrainWaitsForAnOpenStream,
    CancellationReleasesCapacity,
    DrainedClientRefusesNewWork,
    IdleConnectionsAreCapped,
    TrickledBodyPastTheDeadline,
    RedirectIsNotFollowed,
    RefusedStreamIsNotRetried,
    MockedNetworkStaysOnTheHost,
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

async fn listen() -> (TcpListener, Url) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    (listener, url)
}

const TRICKLE_DEADLINE: Duration = Duration::from_millis(300);

// Headers arrive at once, then one body byte every 100 ms: only a deadline that outlives the headers can stop it.
async fn drip(mut stream: TcpStream) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    stream
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n")
        .await
        .unwrap();
    for _ in 0_u8..100 {
        sleep(Duration::from_millis(100)).await;
        if stream.write_all(b"x").await.is_err() {
            return;
        }
    }
}

async fn trickle(listener: TcpListener) {
    let mut open = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        open.spawn(drip(stream));
    }
}

async fn trickled(client: &ProxiedClient) -> String {
    let (listener, url) = listen().await;
    let mut server = JoinSet::new();
    server.spawn(trickle(listener));
    let request = || OutboundRequest {
        deadline: TRICKLE_DEADLINE,
        ..get(&url)
    };
    let buffered = client.send_buffered(request()).await.unwrap_err();
    let streamed = client
        .send(request())
        .await
        .unwrap()
        .body_stream()
        .skip_while(|chunk| ready(chunk.is_ok()))
        .next()
        .await
        .unwrap()
        .unwrap_err();
    let timed_out = matches!(&streamed, HttpError::Request(source) if source.is_timeout());
    format!("{} timed_out={timed_out}", kind(&buffered))
}

// Each reset stream counts once, so a client that retries a REFUSED_STREAM shows up as a second stream.
async fn refuse(listener: TcpListener, streams: Arc<AtomicUsize>) {
    let mut open = JoinSet::new();
    while let Ok((socket, _)) = listener.accept().await {
        let counted = Arc::clone(&streams);
        open.spawn(async move {
            let mut connection = handshake(socket).await.unwrap();
            while let Some(Ok((_request, mut respond))) = connection.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                respond.send_reset(h2::Reason::REFUSED_STREAM);
            }
        });
    }
}

async fn refused() -> String {
    let (listener, url) = listen().await;
    let streams = Arc::new(AtomicUsize::new(0));
    let mut server = JoinSet::new();
    server.spawn(refuse(listener, Arc::clone(&streams)));
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::build(
        base_builder().http2_prior_knowledge(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .unwrap();
    let failure = client.send(get(&url)).await.err().unwrap();
    format!(
        "{} streams={}",
        kind(&failure),
        streams.load(Ordering::SeqCst)
    )
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
    let (listener, url) = listen().await;
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
#[case::a_body_trickling_past_the_deadline_times_out(
    Scenario::TrickledBodyPastTheDeadline,
    "Deadline timed_out=true"
)]
#[case::a_redirect_is_returned_and_not_followed(Scenario::RedirectIsNotFollowed, "302 targets=0")]
#[case::a_refused_http2_stream_is_not_retried(
    Scenario::RefusedStreamIsNotRetried,
    "Request streams=1"
)]
#[case::a_mocked_network_answers_without_connecting(
    Scenario::MockedNetworkStaysOnTheHost,
    "200 chatcmpl-mock-true conn=0 Mock"
)]
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
        Scenario::TrickledBodyPastTheDeadline => trickled(&client).await,
        Scenario::RedirectIsNotFollowed => {
            let target = serve(reply(&["200 OK", "content-length: 0"], b""), Duration::ZERO).await;
            let moved = serve(
                reply(
                    &[
                        "302 Found",
                        &format!("location: {}", target.base),
                        "content-length: 0",
                    ],
                    b"",
                ),
                Duration::ZERO,
            )
            .await;
            let answer = client.send_buffered(get(&moved.base)).await.unwrap();
            format!(
                "{} targets={}",
                answer.status,
                target.connections.load(Ordering::SeqCst)
            )
        }
        Scenario::RefusedStreamIsNotRetried => refused().await,
        Scenario::MockedNetworkStaysOnTheHost => {
            let mocked = ProxiedClient {
                network_mock: true,
                ..client.clone()
            };
            let answer = mocked.send_buffered(get(&slow.base)).await.unwrap();
            let text = String::from_utf8(answer.body).unwrap();
            let listed = text.contains("chatcmpl-mock-");
            let scalar = OutboundRequest {
                body: RequestBody::Json(b"42".to_vec()),
                ..get(&slow.base)
            };
            let refused = mocked.send(scalar).await.err().unwrap();
            format!(
                "{} chatcmpl-mock-{listed} conn={} {}",
                answer.status,
                slow.connections.load(Ordering::SeqCst),
                kind(&refused)
            )
        }
    };
    assert_eq!(observed, expected);
}
