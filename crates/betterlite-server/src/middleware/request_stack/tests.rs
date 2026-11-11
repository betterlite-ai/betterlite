#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::routing::{get, post};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_root_paths::Settings as RootSettings;
use betterlite_config::sections::request_size_limit::Settings as SizeSettings;
use betterlite_config::sections::security_headers::Settings as HeadersSettings;
use betterlite_redis::request_batch::scope::ACTIVE_REQUEST_BATCHES;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use tower::ServiceExt as _;

use crate::middleware::request_root_paths::RootPaths;
use crate::server::application;
use crate::server::chat_setup::chat_executor;

use super::wrap;

const MIB: usize = 1024 * 1024;

fn sizes(max_request_size_mb: Option<f64>) -> SizeSettings {
    SizeSettings {
        max_request_size_mb,
    }
}

fn security(enable_hsts: bool) -> HeadersSettings {
    HeadersSettings { enable_hsts }
}

fn echo_bytes() -> Router {
    Router::new().route(
        "/echo",
        post(|body: Bytes| async move { body.len().to_string() }),
    )
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body.to_vec())
}

fn post_body(body: Vec<u8>) -> Request<Body> {
    Request::post("/echo").body(Body::from(body)).unwrap()
}

#[rstest]
#[case::unset(None)]
#[case::zero(Some(0.0_f64))]
#[case::negative(Some(-1.0_f64))]
#[tokio::test]
async fn without_a_positive_limit_axums_two_mib_limit_stays(#[case] setting: Option<f64>) {
    let app = wrap(echo_bytes(), &sizes(setting), &security(false));
    let oversized = vec![b'a'; 2 * MIB + 1];
    let (status, _headers, body) = send(app, post_body(oversized)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        !body
            .windows(b"Request size is too large".len())
            .any(|window| window == b"Request size is too large"),
        "the body limit must be axum's own, not the size layer's"
    );
}

#[tokio::test]
async fn a_configured_limit_lifts_axums_two_mib_limit() {
    let app = wrap(echo_bytes(), &sizes(Some(10.0_f64)), &security(false));
    let (status, _headers, body) = send(app, post_body(vec![b'a'; 5 * MIB])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, (5 * MIB).to_string().as_bytes());
}

#[tokio::test]
async fn a_body_over_the_configured_limit_gets_a_bare_413() {
    let app = wrap(echo_bytes(), &sizes(Some(1.0_f64)), &security(true));
    let request = Request::post("/echo")
        .header(header::CONTENT_LENGTH, (2 * MIB).to_string())
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(Body::from(vec![b'a'; 2 * MIB]))
        .unwrap();
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body,
        br#"{"error":"Request size is too large. Max size is 1 MB"}"#
    );
    assert_eq!(
        headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes),
        Some(&b"application/json"[..])
    );
    assert_eq!(headers.get(header::CONTENT_ENCODING), None);
    assert_eq!(headers.get(header::X_FRAME_OPTIONS), None);
    assert_eq!(headers.get(header::CONTENT_SECURITY_POLICY), None);
    assert_eq!(headers.get(header::X_CONTENT_TYPE_OPTIONS), None);
}

#[tokio::test]
async fn a_gzipped_response_carries_the_security_headers() {
    let app = wrap(
        Router::new().route("/big", get(|| async { "a".repeat(600) })),
        &sizes(None),
        &security(true),
    );
    let request = Request::get("/big")
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        headers[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000; includeSubDomains"
    );
}

#[tokio::test]
async fn an_exact_root_redirect_carries_the_security_headers() {
    let stacked = wrap(
        Router::new().route("/health/liveliness", get(|| async { "ok" })),
        &sizes(None),
        &security(false),
    );
    let root_settings = RootSettings {
        server_root_path: "/proxy".to_owned(),
        server_root_paths: Vec::new(),
    };
    let app = RootPaths::from_settings(&root_settings)
        .unwrap()
        .wrap(stacked);
    let request = Request::builder()
        .method(Method::GET)
        .uri("/proxy")
        .header(header::HOST, "proxy.example")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(headers[header::LOCATION], "http://proxy.example/proxy/");
    assert!(body.is_empty());
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

#[tokio::test]
async fn the_redis_batch_layer_scopes_the_request() {
    let app = wrap(
        Router::new().route(
            "/scope",
            get(|| async {
                ACTIVE_REQUEST_BATCHES
                    .try_with(|_batches| ())
                    .is_ok()
                    .to_string()
            }),
        ),
        &sizes(None),
        &security(false),
    );
    let request = Request::get("/scope").body(Body::empty()).unwrap();
    let (status, _headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"true");
}

#[tokio::test]
async fn a_request_without_a_root_marker_reaches_the_router() {
    let app = wrap(
        Router::new().route("/plain", get(|| async { "ok" })),
        &sizes(None),
        &security(false),
    );
    let request = Request::get("/plain").body(Body::empty()).unwrap();
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
}

fn server_application(source: &str, environment: &[(&str, &str)]) -> Router {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader {
        lookup: lookup.clone(),
    }
    .load(source)
    .unwrap();
    let executor = chat_executor(&config, &lookup, StdRng::seed_from_u64(1)).unwrap();
    application(&config, executor, Arc::new(AtomicBool::new(false))).unwrap()
}

#[tokio::test]
async fn the_server_wires_the_size_limit_from_config() {
    let app = server_application("general_settings:\n  max_request_size_mb: 1\n", &[]);
    let request = Request::post("/v1/chat/completions")
        .header(header::CONTENT_LENGTH, "2097152")
        .body(Body::from(vec![b'a'; 2 * MIB]))
        .unwrap();
    let (status, headers, body) = send(app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes),
        Some(&b"application/json"[..])
    );
    assert_eq!(
        body,
        br#"{"error":"Request size is too large. Max size is 1 MB"}"#
    );
}

#[tokio::test]
async fn the_server_wires_the_security_headers_from_config() {
    let app = server_application("{}", &[("LITELLM_ENABLE_HSTS", "true")]);
    let request = Request::get("/health/liveliness")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000; includeSubDomains"
    );
}
