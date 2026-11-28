#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use super::support::{FRESH, backup_post, backup_posts, feed, post};
use crate::public_endpoints::blog_feed::testing::FixedClock;
use crate::public_endpoints::blog_feed::{FETCH_TIMEOUT, MAX_FEED_BYTES};

struct Server {
    listener: Arc<TcpListener>,
    url: String,
}

async fn bind() -> Server {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    Server {
        listener: Arc::new(listener),
        url: format!("http://{address}/rss.xml"),
    }
}

fn client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder().http1_only(), &policy, limits).unwrap()
}

async fn read_head(socket: &mut TcpStream) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn answer(code: u16, reason: &str, body: &str) -> String {
    let length = body.len();
    format!(
        "HTTP/1.1 {code} {reason}\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n{body}"
    )
}

fn redirect(location: &str) -> String {
    let head = format!("HTTP/1.1 302 Found\r\nlocation: {location}");
    format!("{head}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
}

async fn serve(listener: Arc<TcpListener>, response: String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    read_head(&mut socket).await;
    socket.write_all(response.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}

async fn serve_counted(listener: Arc<TcpListener>, response: String, hits: Arc<AtomicUsize>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    hits.fetch_add(1, Ordering::SeqCst);
    read_head(&mut socket).await;
    socket.write_all(response.as_bytes()).await.unwrap();
}

async fn hang(listener: Arc<TcpListener>) {
    let (mut socket, _) = listener.accept().await.unwrap();
    read_head(&mut socket).await;
    sleep(Duration::from_secs(20)).await;
}

#[tokio::test]
async fn five_second_deadline_ends_a_hanging_fetch() {
    let server = bind().await;
    let mut tasks = JoinSet::new();
    tasks.spawn(hang(Arc::clone(&server.listener)));
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(
        client(TransportLimits::DEFAULT),
        &server.url,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let started = Instant::now();
    let posts = feed.get_blog_posts().await.unwrap();
    let elapsed = started.elapsed();
    assert_eq!(posts, vec![backup_post()]);
    assert!(elapsed >= FETCH_TIMEOUT);
    assert!(elapsed < Duration::from_secs(15));
    assert_eq!(warnings.warnings().len(), 1);
}

#[tokio::test]
async fn rejects_a_non_success_status() {
    let server = bind().await;
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(
        Arc::clone(&server.listener),
        answer(503, "Service Unavailable", FRESH),
    ));
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, warnings) = feed(
        client(TransportLimits::DEFAULT),
        &server.url,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    let recorded = warnings.warnings();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].subject.as_deref(), Some(server.url.as_str()));
    assert!(recorded[0].message.contains("returned HTTP status 503"));
}

#[tokio::test]
async fn does_not_follow_a_redirect() {
    let redirector = bind().await;
    let target = bind().await;
    let location = target.url.clone();
    let hits = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(Arc::clone(&redirector.listener), redirect(&location)));
    tasks.spawn(serve_counted(
        Arc::clone(&target.listener),
        answer(200, "OK", FRESH),
        Arc::clone(&hits),
    ));
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, _) = feed(
        client(TransportLimits::DEFAULT),
        &redirector.url,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn caps_the_response_body() {
    let server = bind().await;
    let oversized = MAX_FEED_BYTES.checked_add(1).unwrap();
    let body = super::capped_feed(oversized);
    let mut tasks = JoinSet::new();
    tasks.spawn(serve(
        Arc::clone(&server.listener),
        answer(200, "OK", &body),
    ));
    let clock = Arc::new(FixedClock { now: 100.0 });
    let limits = TransportLimits {
        body_bytes: MAX_FEED_BYTES.checked_mul(2).unwrap(),
        ..TransportLimits::DEFAULT
    };
    let (feed, warnings) = feed(
        client(limits),
        &server.url,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![backup_post()]);
    let recorded = warnings.warnings();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0]
            .message
            .contains(&format!("larger than {MAX_FEED_BYTES} bytes"))
    );
}

#[tokio::test]
async fn cancelling_a_fetch_does_not_wedge_the_feed() {
    let server = bind().await;
    let (ready, mut accepted) = mpsc::channel::<()>(1);
    let listener = Arc::clone(&server.listener);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        read_head(&mut first).await;
        ready.send(()).await.unwrap();
        let (mut second, _) = listener.accept().await.unwrap();
        read_head(&mut second).await;
        second
            .write_all(answer(200, "OK", FRESH).as_bytes())
            .await
            .unwrap();
        second.shutdown().await.unwrap();
    });
    let clock = Arc::new(FixedClock { now: 100.0 });
    let (feed, _) = feed(
        client(TransportLimits::DEFAULT),
        &server.url,
        false,
        clock,
        Arc::new(backup_posts()),
    );
    let mut fetch = Box::pin(feed.get_blog_posts());
    accepted.recv().await.unwrap();
    timeout(Duration::from_millis(50), &mut fetch)
        .await
        .expect_err("the fetch answered before the server held it");
    drop(fetch);
    let posts = feed.get_blog_posts().await.unwrap();
    assert_eq!(posts, vec![post("Fresh", "https://fresh")]);
}
