#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{File, read_to_string};
use std::future::pending;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, Method};
use axum::routing::get;
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
use tokio::time::{sleep, timeout};
use tracing::subscriber::set_default;
use url::Url;

use super::chat_setup::chat_executor;
use super::component::serve_component;
use super::connections::{
    CONNECTION_DRAIN_TIMEOUT, configure_connection_builder, serve_connection,
};
use super::filter::ComponentRoutes;
use super::{OUTBOUND_DRAIN_TIMEOUT, run_with_listener};

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

// the listener is bound before the signal handler, so only the listening line says SIGTERM is handled
async fn shut_down(
    log: &Path,
    held: StreamResponse,
    release: Option<Duration>,
) -> Option<StreamResponse> {
    while !read_to_string(log)
        .unwrap()
        .contains("betterlite server listening")
    {
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
    // startup alone takes seconds on a loaded runner, so the ceiling only turns a hung shutdown into a failure
    let limit = CONNECTION_DRAIN_TIMEOUT
        .saturating_add(OUTBOUND_DRAIN_TIMEOUT)
        .saturating_add(Duration::from_secs(30));
    let (served, _kept) = tokio::join!(
        timeout(
            limit,
            run_with_listener(config, chat, listener, "127.0.0.1")
        ),
        shut_down(&path, held, release)
    );
    served.unwrap().unwrap();
    let refused = outbound.send(request()).await.err().unwrap();
    assert_eq!(format!("{refused:?}"), "Closed");
    let written = read_to_string(&path).unwrap();
    // the warning is the drain's outcome: it fires only when a request is still open at the deadline
    assert_eq!(
        written.contains("outbound requests did not finish before shutdown"),
        warned,
        "{written}"
    );
}

// the server answers one request per connection, so a client that asks for keep-alive still reads to the end
#[tokio::test]
async fn a_connection_closes_after_its_first_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/", get(|| async { "ok" }));
    let mut serving = JoinSet::new();
    serving.spawn(async move {
        let (stream, remote) = listener.accept().await.unwrap();
        serve_connection(configure_connection_builder(), router, stream, remote).await;
    });
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut raw = String::new();
    timeout(Duration::from_secs(5), client.read_to_string(&mut raw))
        .await
        .unwrap()
        .unwrap();
    assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"), "{raw}");
    assert!(raw.ends_with("\r\n\r\nok"), "{raw}");
    serving.join_next().await.unwrap().unwrap();
}

static NO_ROUTES: phf::Set<&'static str> = phf::phf_set! { "/unrouted" };

// a config that cannot be read stops startup with a failure code before anything binds
#[test]
fn a_component_without_a_readable_config_exits_with_failure() {
    let folder = tempfile::tempdir().unwrap();
    let routes = ComponentRoutes {
        prefixes: &[],
        exact: &NO_ROUTES,
        mounts: &NO_ROUTES,
    };
    let code = serve_component(&folder.path().join("missing.yaml"), "127.0.0.1", 0, routes);
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
}
