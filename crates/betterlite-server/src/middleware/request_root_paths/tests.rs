#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::OriginalUri;
use axum::http::{Extensions, Method, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_root_paths::Settings;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use tokio::sync::Barrier;
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tower::ServiceExt as _;
use tracing::subscriber::with_default;

use super::normalize::normalize_root_paths;
use super::{RootPaths, get_request_root_path};
use crate::server::application;
use crate::server::chat_setup::chat_executor;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Seen {
    path: String,
    original: String,
    root: String,
}

fn settings(scalar_root: &str, prefixes: &[&str]) -> Settings {
    Settings {
        server_root_path: scalar_root.to_owned(),
        server_root_paths: prefixes.iter().map(|prefix| (*prefix).to_owned()).collect(),
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn echo_app(scalar_root: &str) -> Router {
    let fallback_root = scalar_root.to_owned();
    Router::new().fallback(move |request: Request<Body>| async move {
        let original = request.extensions().get::<OriginalUri>().unwrap();
        Json(Seen {
            path: request.uri().to_string(),
            original: original.0.to_string(),
            root: get_request_root_path(request.extensions(), &fallback_root),
        })
    })
}

fn wrapped(scalar_root: &str, prefixes: &[&str]) -> Router {
    let paths = RootPaths::from_settings(&settings(scalar_root, prefixes)).unwrap();
    paths.wrap(echo_app(scalar_root))
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, body.to_vec())
}

async fn seen(app: &Router, uri: &str) -> Seen {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let (status, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_slice(&body).unwrap()
}

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, read_to_string(log.path()).unwrap())
}

#[rstest]
#[case::trailing_slash(&["/a/"], &["/a"])]
#[case::surrounding_whitespace(&["  /a  "], &["/a"])]
#[case::repeated_trailing_slashes(&["/a///"], &["/a"])]
#[case::empty_entries(&["", "   ", "/a"], &["/a"])]
#[case::duplicates_after_trimming(&["/a", "/a/", " /a "], &["/a"])]
#[case::longest_first(&["/a", "/a/b/c", "/a/b"], &["/a/b/c", "/a/b", "/a"])]
#[case::stable_ties(&["/bb", "/aa", "/cc"], &["/bb", "/aa", "/cc"])]
#[case::nothing_left(&[], &[])]
fn normalize_root_paths_cleans_entries(#[case] raw: &[&str], #[case] expected: &[&str]) {
    let (normalized, _log) = capture_warnings(|| normalize_root_paths(&strings(raw)));
    assert_eq!(normalized, strings(expected));
}

#[rstest]
#[case::missing_slash(&["tenant-a", "/b"], &["/b"], "tenant-a")]
#[case::bare_root(&["/", "/b"], &["/b"], "bare root")]
#[case::root_slashes(&["///", "/b"], &["/b"], "bare root")]
fn normalize_root_paths_warns_and_drops(
    #[case] raw: &[&str],
    #[case] expected: &[&str],
    #[case] mention: &str,
) {
    let (normalized, log) = capture_warnings(|| normalize_root_paths(&strings(raw)));
    assert_eq!(normalized, strings(expected));
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains(mention), "{log}");
}

#[test]
fn normalize_root_paths_stays_quiet_for_valid_entries() {
    let (_normalized, log) = capture_warnings(|| normalize_root_paths(&strings(&["/a", " /b/ "])));
    assert_eq!(log, "");
}

#[test]
fn request_root_path_falls_back_to_the_scalar_root() {
    let extensions = Extensions::new();
    assert_eq!(get_request_root_path(&extensions, "/gateway"), "/gateway");
    assert_eq!(get_request_root_path(&extensions, ""), "");
}

#[rstest]
#[case::matched("/tenant-a/x", "/tenant-a")]
#[case::unmatched_keeps_scalar("/other/x", "/gateway")]
#[tokio::test]
async fn request_root_path_reads_the_stashed_root(#[case] uri: &str, #[case] expected: &str) {
    let app = wrapped("/gateway", &["/tenant-a"]);
    assert_eq!(seen(&app, uri).await.root, expected);
}

#[tokio::test]
async fn request_root_path_stashed_empty_root_beats_the_scalar_argument() {
    let app = RootPaths::from_settings(&settings("", &["/tenant-a"]))
        .unwrap()
        .wrap(echo_app("/caller-scalar"));
    assert_eq!(seen(&app, "/other").await.root, "");
}

#[rstest]
#[case::exact_prefix("/tenant-a", "/tenant-a", "/")]
#[case::trailing_slash("/tenant-a/", "/tenant-a", "/")]
#[case::below_prefix("/tenant-a/health/liveliness", "/tenant-a", "/health/liveliness")]
#[case::query_kept("/tenant-a/models?limit=2", "/tenant-a", "/models?limit=2")]
#[case::exact_prefix_query_kept("/tenant-a?x=1", "/tenant-a", "/?x=1")]
#[case::sibling_prefix_rejected("/tenant-ab/models", "", "/tenant-ab/models")]
#[case::unmatched_untouched("/v1/models", "", "/v1/models")]
#[case::prefix_not_at_start("/x/tenant-a/models", "", "/x/tenant-a/models")]
#[tokio::test]
async fn root_path_middleware_matches_whole_segments(
    #[case] uri: &str,
    #[case] root: &str,
    #[case] routed: &str,
) {
    let app = wrapped("", &["/tenant-a"]);
    let observed = seen(&app, uri).await;
    assert_eq!(observed.root, root);
    assert_eq!(observed.path, routed);
    assert_eq!(observed.original, uri);
}

#[tokio::test]
async fn root_path_middleware_does_not_match_a_percent_encoded_prefix() {
    let app = wrapped("", &["/tenant-a"]);
    let observed = seen(&app, "/tenant%2Da/models").await;
    assert_eq!(observed.root, "");
    assert_eq!(observed.path, "/tenant%2Da/models");
}

#[rstest]
#[case::inner_wins("/a/b/models", "/a/b", "/models")]
#[case::outer_for_other_child("/a/c/models", "/a", "/c/models")]
#[case::inner_exact("/a/b", "/a/b", "/")]
#[case::sibling_of_inner("/a/bc", "/a", "/bc")]
#[tokio::test]
async fn root_path_middleware_prefers_the_longest_prefix(
    #[case] uri: &str,
    #[case] root: &str,
    #[case] routed: &str,
) {
    let app = wrapped("", &["/a", "/a/b"]);
    let observed = seen(&app, uri).await;
    assert_eq!(observed.root, root);
    assert_eq!(observed.path, routed);
}

#[rstest]
#[case::scalar_stripped("/gateway/models", "/gateway", "/models")]
#[case::scalar_exact("/gateway", "/gateway", "/")]
#[case::scalar_sibling("/gatewayx/models", "/gateway", "/gatewayx/models")]
#[case::scalar_absent("/models", "/gateway", "/models")]
#[case::prefix_beats_scalar("/tenant-a/gateway/models", "/tenant-a", "/gateway/models")]
#[tokio::test]
async fn root_path_middleware_unmatched_requests_keep_the_scalar_root(
    #[case] uri: &str,
    #[case] root: &str,
    #[case] routed: &str,
) {
    let app = wrapped("/gateway", &["/tenant-a"]);
    let observed = seen(&app, uri).await;
    assert_eq!(observed.root, root);
    assert_eq!(observed.path, routed);
}

#[test]
fn root_path_middleware_installed_only_for_a_nonempty_prefix_list() {
    assert!(RootPaths::from_settings(&settings("", &[])).is_none());
    assert!(RootPaths::from_settings(&settings("/gateway", &[])).is_none());
    assert!(RootPaths::from_settings(&settings("", &["", "tenant-a", "/"])).is_none());
    assert!(RootPaths::from_settings(&settings("", &["/tenant-a"])).is_some());
}

#[rstest]
#[case::list_unset("{}", StatusCode::NOT_FOUND)]
#[case::list_set("general_settings:\n  SERVER_ROOT_PATHS: /tenant-a\n", StatusCode::OK)]
#[tokio::test]
async fn root_path_middleware_is_installed_by_the_server_only_for_a_prefix_list(
    #[case] source: &str,
    #[case] expected: StatusCode,
) {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    let application = application(&config, executor, Arc::new(AtomicBool::new(false))).unwrap();
    let request = Request::builder()
        .uri("/tenant-a/health/liveliness")
        .body(Body::empty())
        .unwrap();
    let (status, _body) = send(&application, request).await;
    assert_eq!(status, expected);
}

#[rstest]
#[case::http(Method::GET, false)]
#[case::post(Method::POST, false)]
#[case::websocket_upgrade(Method::GET, true)]
#[tokio::test]
async fn scope_type_http_and_websocket_requests_resolve_the_prefix(
    #[case] method: Method,
    #[case] upgrade: bool,
) {
    let app = wrapped("", &["/tenant-a"]);
    let mut builder = Request::builder().method(method).uri("/tenant-a/ws");
    if upgrade {
        builder = builder
            .header("connection", "upgrade")
            .header("upgrade", "websocket");
    }
    let (status, body) = send(&app, builder.body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    let observed: Seen = serde_json::from_slice(&body).unwrap();
    assert_eq!(observed.root, "/tenant-a");
    assert_eq!(observed.path, "/ws");
}

#[tokio::test]
async fn scope_type_connect_requests_pass_through_untouched() {
    let app = RootPaths::from_settings(&settings("/gateway", &["/tenant-a"]))
        .unwrap()
        .wrap(Router::new().fallback(|request: Request<Body>| async move {
            [
                ("x-seen-uri", request.uri().to_string()),
                (
                    "x-seen-root",
                    get_request_root_path(request.extensions(), "/unset"),
                ),
            ]
        }));
    let request = Request::builder()
        .method(Method::CONNECT)
        .uri("tenant-a:443")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-seen-uri"], "tenant-a:443");
    assert_eq!(response.headers()["x-seen-root"], "/unset");
}

#[tokio::test]
async fn context_isolation_concurrent_requests_keep_their_own_root() {
    let barrier = Arc::new(Barrier::new(2));
    let app = RootPaths::from_settings(&settings("", &["/tenant-a", "/tenant-b"]))
        .unwrap()
        .wrap(Router::new().fallback(move |request: Request<Body>| {
            let waiting = Arc::clone(&barrier);
            async move {
                let before = get_request_root_path(request.extensions(), "none");
                waiting.wait().await;
                let after = get_request_root_path(request.extensions(), "none");
                format!("{before}|{after}")
            }
        }));
    let first_request = Request::builder()
        .uri("/tenant-a/x")
        .body(Body::empty())
        .unwrap();
    let second_request = Request::builder()
        .uri("/tenant-b/x")
        .body(Body::empty())
        .unwrap();
    let ((_, first), (_, second)) =
        tokio::join!(send(&app, first_request), send(&app, second_request));
    assert_eq!(first, b"/tenant-a|/tenant-a");
    assert_eq!(second, b"/tenant-b|/tenant-b");
}

#[tokio::test]
async fn context_isolation_next_request_starts_from_the_scalar_root() {
    let app = wrapped("/gateway", &["/tenant-a"]);
    assert_eq!(seen(&app, "/tenant-a/x").await.root, "/tenant-a");
    assert_eq!(seen(&app, "/other").await.root, "/gateway");
    assert_eq!(
        get_request_root_path(&Extensions::new(), "/gateway"),
        "/gateway"
    );
}

async fn failing_or_hanging(request: Request<Body>) -> (StatusCode, String) {
    match request.uri().path() {
        "/boom" => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        "/hang" => pending().await,
        _ => (
            StatusCode::OK,
            get_request_root_path(request.extensions(), "none"),
        ),
    }
}

#[tokio::test]
async fn context_isolation_failure_and_cancellation_leave_later_requests_alone() {
    let app = RootPaths::from_settings(&settings("/gateway", &["/tenant-a"]))
        .unwrap()
        .wrap(Router::new().fallback(failing_or_hanging));
    let failing = Request::builder()
        .uri("/tenant-a/boom")
        .body(Body::empty())
        .unwrap();
    let (status, _body) = send(&app, failing).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    let hanging = Request::builder()
        .uri("/tenant-a/hang")
        .body(Body::empty())
        .unwrap();
    let cancelled = timeout(Duration::from_millis(20), send(&app, hanging)).await;
    assert!(matches!(cancelled, Err(Elapsed { .. })));

    let later = Request::builder()
        .uri("/other")
        .body(Body::empty())
        .unwrap();
    let (_, body) = send(&app, later).await;
    assert_eq!(body, b"/gateway");
}

#[rstest]
#[case::scalar_and_list("/gateway", &["/a"], true)]
#[case::scalar_with_trailing_slash("/gateway/", &["/a"], true)]
#[case::empty_scalar("", &["/a"], false)]
#[case::root_scalar("/", &["/a"], false)]
#[case::no_list("/gateway", &[], false)]
#[case::list_all_dropped("/gateway", &["/"], false)]
fn root_path_warning_when_scalar_and_list_coexist(
    #[case] scalar_root: &str,
    #[case] prefixes: &[&str],
    #[case] warns: bool,
) {
    let (_paths, log) =
        capture_warnings(|| RootPaths::from_settings(&settings(scalar_root, prefixes)));
    if warns {
        assert!(log.contains("WARN"), "{log}");
        assert!(log.contains("are both set"), "{log}");
    } else {
        assert!(!log.contains("are both set"), "{log}");
    }
}
