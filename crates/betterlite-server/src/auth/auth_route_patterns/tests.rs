#![cfg(test)]

use crate::routes::router;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::subscriber::with_default;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use tower::ServiceExt as _;

async fn downstream(State(calls): State<Arc<AtomicUsize>>) -> &'static str {
    calls.fetch_add(1, Ordering::Relaxed);
    "reached"
}

fn configured_router(calls: Arc<AtomicUsize>, public_routes: &str) -> Router {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: secret\n  public_routes: {public_routes}\n"
    ))
    .unwrap();
    let chat = Router::new()
        .route("/v1/chat/completions", get(downstream))
        .route("/v1/completions", get(downstream))
        .with_state(calls);
    router(
        super::AuthSettings {
            master_key: config.general_settings.master_key,
            routes: config.sections.auth_route_patterns,
        },
        Arc::new(AtomicBool::new(false)),
        &config.model_list,
        chat,
        Router::new(),
    )
}

#[tokio::test]
async fn public_stage_reaches_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = configured_router(Arc::clone(&calls), "['/v1/chat/completions']")
        .oneshot(
            Request::get("/v1/chat/completions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn failure_stops_downstream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = configured_router(Arc::clone(&calls), "['/v1/chat/completions']")
        .oneshot(Request::get("/v1/completions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    assert_eq!(&*bytes, b"Internal Server Error");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[rstest]
#[case::native_public_routes_without_license("/v1/chat/completions", "['/v1/chat/*']", 200)]
#[case::metrics_templates_not_auth_aliases(
    "/v1/threads/thread_123",
    "['/v1/threads/{thread_id}']",
    500
)]
#[case::query_not_route("/v1/completions?public=/v1/chat/completions", "['/v1/chat/*']", 500)]
#[case::sibling_root_kept("/v1/completions", "['/v1/chat/*']", 500)]
#[case::malformed_host("/v1/completions", "['/']", 500)]
#[tokio::test]
async fn public_route_dispatch(#[case] path: &str, #[case] routes: &str, #[case] status: u16) {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = configured_router(Arc::clone(&calls), routes)
        .oneshot(
            Request::get(path)
                .header("host", "localhost/?x=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(calls.load(Ordering::Relaxed), usize::from(status == 200));
}

#[test]
fn path_exception_debug_fallback() {
    use betterlite_auth::auth_route_patterns::canonical::{
        RequestRouteSource, ScopeError, get_request_route,
    };
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    with_default(subscriber, || {
        assert_eq!(
            get_request_route(&RequestRouteSource {
                scope: Err(ScopeError::Access),
                url_path: "/fallback"
            }),
            "/fallback"
        );
    });
    let output = read_to_string(log.path()).unwrap();
    assert!(output.contains("DEBUG"));
    assert!(output.contains("using URL path"));
}

#[test]
fn template_exception_debug_none() {
    use betterlite_auth::auth_route_patterns::canonical::{ScopeError, get_request_route_template};
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    with_default(subscriber, || {
        assert_eq!(get_request_route_template(Err(ScopeError::Access)), None);
    });
    let output = read_to_string(log.path()).unwrap();
    assert!(output.contains("DEBUG"));
    assert!(output.contains("template access failed"));
}

#[test]
fn malformed_public_list_logs_false() {
    use betterlite_auth::auth_route_patterns::public_routes::{
        RoutePatternError, check_configured_public_routes,
    };
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    with_default(subscriber, || {
        assert!(!check_configured_public_routes(
            "/v1/models",
            Err(RoutePatternError::MalformedSettings)
        ));
    });
    let output = read_to_string(log.path()).unwrap();
    assert!(output.contains("ERROR"));
    assert!(output.contains("configured public routes"));
}

#[rstest]
#[case::native_public_routes_without_license(
    "/proxy/v1/chat/completions",
    "['/v1/chat/completions']",
    200
)]
#[case::native_general_settings_public_routes_works_without_premium_user(
    "/proxy/v1/chat/completions",
    "['/v1/chat/*']",
    200
)]
#[case::root_whole_segment("/proxy/v1/completions", "['/v1/chat/*']", 500)]
#[case::sibling_root_kept("/proxy-other/v1/chat/completions", "['/v1/chat/*']", 404)]
#[case::malformed_host("/proxy/v1/completions", "['/']", 500)]
#[tokio::test]
async fn booted_server(#[case] path: &str, #[case] routes: &str, #[case] status: u16) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let calls = Arc::new(AtomicUsize::new(0));
    let application = Router::new().nest("/proxy", configured_router(Arc::clone(&calls), routes));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        axum::serve(listener, application)
            .with_graceful_shutdown(async {
                let _result = stopped.await;
            })
            .await
            .unwrap();
    });
    let wire = timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost/?x=1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    })
    .await
    .unwrap();
    stop.send(()).unwrap();
    tasks.join_next().await.unwrap().unwrap();
    assert!(wire.starts_with(&format!("HTTP/1.1 {status} ")), "{wire}");
    assert_eq!(calls.load(Ordering::Relaxed), usize::from(status == 200));
}

#[tokio::test]
async fn stage_is_registered() {
    use crate::server::{chat_setup::chat_executor, run};
    use rand::SeedableRng as _;
    use rand::rngs::StdRng;
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings: {master_key: secret, public_routes: ['/v1/models']}")
    .unwrap();
    let chat = chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(7)).unwrap();
    let deadline = Duration::from_secs(2);
    let (server, client) = tokio::join!(
        timeout(deadline, run(config, chat, "127.0.0.1", port)),
        timeout(deadline, request_once_ready(port)),
    );
    server.unwrap_err();
    let response = client.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
    assert!(response.contains(r#""object":"list""#));
}

async fn request_once_ready(port: u16) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(connection) => break connection,
            Err(_) => sleep(Duration::from_millis(10)).await,
        }
    };
    stream
        .write_all(b"GET /v1/models HTTP/1.1\r\nHost: localhost/?x=1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .await
        .unwrap();
    response
}

#[tokio::test]
async fn invalid_credentials_keep_public_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let response = configured_router(Arc::clone(&calls), "['/v1/chat/completions']")
        .oneshot(
            Request::get("/v1/completions")
                .header("authorization", "Bearer rejected")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let body: ErrorResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.error.error_type, "no_db_connection");
    assert_eq!(body.error.param, None);
    assert_eq!(body.error.code, "400");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}
