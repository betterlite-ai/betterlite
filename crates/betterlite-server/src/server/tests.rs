#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{File, read_to_string};
use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method};
use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use signal_hook::consts::SIGTERM;
use signal_hook::low_level::raise;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, timeout};
use tracing::subscriber::set_default;
use url::Url;

use super::chat_setup::chat_executor;
use super::{CONNECTION_DRAIN_TIMEOUT, OUTBOUND_DRAIN_TIMEOUT, run};

// Answers with headers and then keeps the response open, so the client holds a lease on it.
async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    stream
        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\n")
        .await
        .unwrap();
    pending::<()>().await;
}

async fn shut_down(
    port: u16,
    held: StreamResponse,
    release: Option<Duration>,
) -> Option<StreamResponse> {
    while TcpStream::connect(("127.0.0.1", port)).await.is_err() {
        sleep(Duration::from_millis(10)).await;
    }
    raise(SIGTERM).unwrap();
    match release {
        Some(after) => {
            sleep(after).await;
            None
        }
        None => Some(held),
    }
}

#[rstest]
#[case::run_waits_for_outbound_work_that_finishes(Some(Duration::from_millis(300)), false)]
#[case::run_gives_up_on_outbound_work_at_the_deadline(None, true)]
#[tokio::test]
async fn http_pool_tests_behavior(#[case] release: Option<Duration>, #[case] warned: bool) {
    let provider = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = Url::parse(&format!("http://{}/", provider.local_addr().unwrap())).unwrap();
    let mut origin = JoinSet::new();
    origin.spawn(hold_open(provider));
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("model_list:\n  - {model_name: g, litellm_params: {model: openai/m, api_key: k}}\n")
    .unwrap();
    let chat = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    let outbound = chat.client.clone();
    let request = || OutboundRequest {
        method: Method::POST,
        url: upstream.clone(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(b"{}".to_vec()),
        deadline: Duration::from_secs(60),
    };
    let held = outbound.send(request()).await.unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("log");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(File::create(&path).unwrap()))
        .finish();
    let _guard = set_default(subscriber);
    let started = Instant::now();
    let limit = CONNECTION_DRAIN_TIMEOUT
        .saturating_add(OUTBOUND_DRAIN_TIMEOUT)
        .saturating_add(Duration::from_secs(1));
    let (served, _kept) = tokio::join!(
        timeout(limit, run(config, chat, "127.0.0.1", port)),
        shut_down(port, held, release)
    );
    let elapsed = started.elapsed();
    served.unwrap().unwrap();
    let refused = outbound.send(request()).await.err().unwrap();
    assert_eq!(format!("{refused:?}"), "Closed");
    let written = read_to_string(&path).unwrap();
    assert_eq!(
        written.contains("outbound requests did not finish before shutdown"),
        warned,
        "{written}"
    );
    assert_eq!(elapsed >= OUTBOUND_DRAIN_TIMEOUT, warned, "{elapsed:?}");
}
