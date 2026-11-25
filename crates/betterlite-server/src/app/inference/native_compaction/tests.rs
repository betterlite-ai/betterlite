#![cfg(test)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, OriginalUri, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use betterlite_anthropic::native_compaction::protocol::{CompactionProtocol, Mapping};
use betterlite_config::sections::request_root_paths::Settings as RootPathSettings;
use betterlite_models::decisions::Json as JsonValue;
use betterlite_router::compaction_executor::CompactionExecutor as _;
use indexmap::IndexMap;
use rstest::rstest;
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tower::ServiceExt as _;

use super::request::executor_for;
use super::target::{LoopbackTarget, install};
use crate::middleware::request_root_paths::RootPaths;
use crate::server::connections::{configure_connection_builder, serve_connection};

const CHILD_MODEL: &str = "compactor-model";
const PARENT_AUTH: &str = "Bearer parent-key";
const PARENT_PEER: &str = "203.0.113.7:4321";

#[derive(Clone, Serialize)]
struct Seen {
    path: String,
    uri: String,
    original: Option<String>,
    authorization: Option<String>,
    peer: Option<SocketAddr>,
}

#[derive(Default)]
struct Shared {
    seen: Mutex<Vec<Seen>>,
}

impl Shared {
    fn recorded(&self) -> Seen {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        seen.pop().expect("the child request was recorded")
    }
}

#[derive(Default)]
struct TargetRecorder {
    targets: Mutex<Vec<Option<Arc<LoopbackTarget>>>>,
}

fn summary_payload() -> Mapping {
    IndexMap::from([
        ("model".to_owned(), JsonValue::Text(CHILD_MODEL.to_owned())),
        (
            "messages".to_owned(),
            JsonValue::List(vec![JsonValue::Object(IndexMap::from([(
                "role".to_owned(),
                JsonValue::Text("user".to_owned()),
            )]))]),
        ),
    ])
}

fn loopback_router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(loopback))
        .route("/v1/messages", post(loopback))
        .with_state(shared)
}

async fn loopback(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    let (parts, _body) = request.into_parts();
    let Some(executor) = executor_for(&parts) else {
        let seen = Seen {
            path: parts.uri.path().to_owned(),
            uri: parts.uri.to_string(),
            original: parts
                .extensions
                .get::<OriginalUri>()
                .map(|original| original.0.to_string()),
            authorization: parts
                .headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            peer: parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connect_info| connect_info.0),
        };
        shared.seen.lock().unwrap().push(seen.clone());
        return Json(seen).into_response();
    };
    let protocol = if parts.uri.path().ends_with("/messages") {
        CompactionProtocol::Messages
    } else {
        CompactionProtocol::Chat
    };
    match executor
        .execute(protocol, &summary_payload(), None, false)
        .await
    {
        Ok(summary) => Json(summary).into_response(),
        Err(error) => (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

fn probe_router(recorder: Arc<TargetRecorder>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(record_target))
        .with_state(recorder)
}

async fn record_target(
    State(recorder): State<Arc<TargetRecorder>>,
    request: Request,
) -> StatusCode {
    let target = request.extensions().get::<Arc<LoopbackTarget>>().cloned();
    recorder.targets.lock().unwrap().push(target);
    StatusCode::NO_CONTENT
}

fn probe() -> Request {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap()
}

async fn report_peer(ConnectInfo(address): ConnectInfo<SocketAddr>) -> String {
    address.to_string()
}

fn request_with_target(mut request: Request, scalar_root: &str) -> Request {
    request.extensions_mut().insert(Arc::new(LoopbackTarget {
        app: Router::new(),
        scalar_root: scalar_root.to_owned(),
    }));
    request
}

#[tokio::test]
async fn target_a_request_through_install_carries_the_target() {
    let recorder = Arc::new(TargetRecorder::default());
    let installed = install(probe_router(Arc::clone(&recorder)), "/gateway".to_owned());
    let response = installed.oneshot(probe()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let target = {
        let mut targets = recorder.targets.lock().unwrap();
        assert_eq!(targets.len(), 1);
        targets
            .pop()
            .expect("the probe request ran")
            .expect("a request through install carries the target")
    };
    assert_eq!(target.scalar_root, "/gateway");
}

#[tokio::test]
async fn target_the_target_app_is_the_router_from_before_install() {
    let recorder = Arc::new(TargetRecorder::default());
    let installed = install(probe_router(Arc::clone(&recorder)), "/gateway".to_owned());
    let response = installed.oneshot(probe()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let target = {
        let mut targets = recorder.targets.lock().unwrap();
        assert_eq!(targets.len(), 1);
        targets
            .pop()
            .unwrap()
            .expect("a request through install carries the target")
    };
    let from_target = target.app.clone().oneshot(probe()).await.unwrap();
    assert_eq!(from_target.status(), StatusCode::NO_CONTENT);
    let without = recorder.targets.lock().unwrap().pop().unwrap();
    assert!(
        without.is_none(),
        "the target app adds no target of its own"
    );
}

#[test]
fn executor_a_request_without_a_target_builds_no_executor() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let (parts, _body) = request.into_parts();
    assert!(executor_for(&parts).is_none());
}

#[test]
fn executor_a_request_without_a_peer_builds_no_peer() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let parent = request_with_target(request, "");
    let (parts, _body) = parent.into_parts();
    let executor = executor_for(&parts).expect("a request with a target builds an executor");
    assert!(
        executor.peer.is_none(),
        "the request carried no peer and the executor gets none"
    );
}

#[test]
fn executor_the_parent_uri_prefers_the_original_uri() {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(OriginalUri(Uri::from_static(
            "https://gateway.internal:4000/v1/chat/completions",
        )));
    let parent = request_with_target(request, "");
    let (parts, _body) = parent.into_parts();
    let executor = executor_for(&parts).expect("a request with a target builds an executor");
    assert_eq!(
        executor.parent_uri,
        Uri::from_static("https://gateway.internal:4000/v1/chat/completions")
    );
}

#[test]
fn executor_the_parent_uri_falls_back_to_the_request_uri() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("https://gateway.internal:4000/v1/chat/completions?stream=true")
        .body(Body::empty())
        .unwrap();
    let parent = request_with_target(request, "");
    let (parts, _body) = parent.into_parts();
    let executor = executor_for(&parts).expect("a request with a target builds an executor");
    assert_eq!(
        executor.parent_uri,
        Uri::from_static("https://gateway.internal:4000/v1/chat/completions?stream=true")
    );
}

#[test]
fn executor_the_root_path_falls_back_to_the_target_scalar_root() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let parent = request_with_target(request, "/gateway");
    let (parts, _body) = parent.into_parts();
    let executor = executor_for(&parts).expect("a request with a target builds an executor");
    assert_eq!(executor.root_path, "/gateway");
}

#[rstest]
#[case::chat("/v1/chat/completions")]
#[case::messages("/v1/messages")]
#[tokio::test]
async fn executor_the_child_reaches_the_protocol_route_with_the_parent_auth_and_peer(
    #[case] path: &'static str,
) {
    let shared = Arc::new(Shared::default());
    let app = install(loopback_router(Arc::clone(&shared)), String::new());
    let peer: SocketAddr = PARENT_PEER.parse().unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(AUTHORIZATION, PARENT_AUTH)
        .extension(ConnectInfo(peer))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let child = shared.recorded();
    assert_eq!(child.path, path);
    assert_eq!(child.authorization.as_deref(), Some(PARENT_AUTH));
    assert_eq!(child.peer, Some(peer));
}

#[tokio::test]
async fn executor_the_child_passes_a_server_root_paths_prefix() {
    let shared = Arc::new(Shared::default());
    let settings = RootPathSettings {
        server_root_path: String::new(),
        server_root_paths: vec!["/tenant-a".to_owned()],
    };
    let paths = RootPaths::from_settings(&settings).expect("the prefix installs the middleware");
    let app = install(
        paths.wrap(loopback_router(Arc::clone(&shared))),
        String::new(),
    );
    let peer: SocketAddr = PARENT_PEER.parse().unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/tenant-a/v1/chat/completions")
        .header(AUTHORIZATION, PARENT_AUTH)
        .extension(ConnectInfo(peer))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let child = shared.recorded();
    assert_eq!(child.path, "/v1/chat/completions");
    assert_eq!(
        child.original.as_deref(),
        Some("/tenant-a/v1/chat/completions")
    );
    assert_eq!(child.authorization.as_deref(), Some(PARENT_AUTH));
    assert_eq!(child.peer, Some(peer));
}

#[tokio::test]
async fn executor_the_child_uses_the_scalar_root_path_when_no_prefix_matches() {
    let shared = Arc::new(Shared::default());
    let settings = RootPathSettings {
        server_root_path: "/gateway".to_owned(),
        server_root_paths: Vec::new(),
    };
    let paths =
        RootPaths::from_settings(&settings).expect("the scalar root installs the middleware");
    let app = install(
        paths.wrap(loopback_router(Arc::clone(&shared))),
        "/gateway".to_owned(),
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/gateway/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let child = shared.recorded();
    assert_eq!(child.path, "/v1/chat/completions");
    assert_eq!(
        child.original.as_deref(),
        Some("/gateway/v1/chat/completions")
    );
}

#[tokio::test]
async fn peer_serve_connection_puts_the_tcp_peer_on_the_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route("/peer", get(report_peer));
    let mut serving = JoinSet::new();
    serving.spawn(async move {
        let (stream, remote) = listener.accept().await.unwrap();
        serve_connection(configure_connection_builder(), app, stream, remote).await;
    });
    let mut client = TcpStream::connect(address).await.unwrap();
    let local = client.local_addr().unwrap();
    client
        .write_all(b"GET /peer HTTP/1.1\r\nhost: gateway\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut raw = String::new();
    timeout(Duration::from_secs(5), client.read_to_string(&mut raw))
        .await
        .unwrap()
        .unwrap();
    serving.join_next().await.unwrap().unwrap();
    let body = raw
        .split_once("\r\n\r\n")
        .expect("the reply carries a body")
        .1;
    assert_eq!(body, local.to_string());
}
