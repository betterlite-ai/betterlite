#![cfg(test)]

use std::convert::Infallible;
use std::future::{Ready, ready};
use std::task::{Context, Poll, Waker};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use rstest::rstest;
use tower::{Layer as _, Service, ServiceExt as _};

use super::SecurityHeadersLayer;

async fn send(app: Router, method: Method, uri: &str) -> Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

fn routed(enable_hsts: bool) -> Router {
    Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(SecurityHeadersLayer { enable_hsts })
}

fn setting_frame_options() -> Router {
    Router::new()
        .route(
            "/ok",
            get(|| async {
                let mut response = Response::new(Body::from("ok"));
                response.headers_mut().insert(
                    header::X_FRAME_OPTIONS,
                    HeaderValue::from_static("SAMEORIGIN"),
                );
                response
            }),
        )
        .layer(SecurityHeadersLayer { enable_hsts: false })
}

fn handshake(enable_hsts: bool) -> Router {
    Router::new()
        .route(
            "/ws",
            get(|| async { (StatusCode::SWITCHING_PROTOCOLS, Body::empty()) }),
        )
        .layer(SecurityHeadersLayer { enable_hsts })
}

fn assert_static_headers(headers: &HeaderMap) {
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

#[tokio::test]
async fn the_static_headers_are_set_on_a_200_response() {
    let response = send(routed(false), Method::GET, "/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_static_headers(response.headers());
    assert_eq!(
        response.headers().get(header::STRICT_TRANSPORT_SECURITY),
        None
    );
}

#[rstest]
#[case::not_found(Method::GET, "/missing", StatusCode::NOT_FOUND)]
#[case::method_not_allowed(Method::POST, "/ok", StatusCode::METHOD_NOT_ALLOWED)]
#[tokio::test]
async fn the_static_headers_are_set_on_every_router_answer(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] status: StatusCode,
) {
    let response = send(routed(true), method, uri).await;
    assert_eq!(response.status(), status);
    assert_static_headers(response.headers());
    assert_eq!(
        response.headers()[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000; includeSubDomains"
    );
}

#[tokio::test]
async fn a_header_the_route_set_itself_is_kept() {
    let response = send(setting_frame_options(), Method::GET, "/ok").await;
    assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "SAMEORIGIN");
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
}

#[rstest]
#[case::enabled(true, Some("max-age=31536000; includeSubDomains"))]
#[case::disabled(false, None)]
#[tokio::test]
async fn hsts_appears_only_when_enabled(#[case] enable_hsts: bool, #[case] expected: Option<&str>) {
    let response = send(routed(enable_hsts), Method::GET, "/ok").await;
    assert_eq!(
        response
            .headers()
            .get(header::STRICT_TRANSPORT_SECURITY)
            .map(|value| value.to_str().unwrap()),
        expected
    );
}

#[tokio::test]
async fn a_switching_protocols_handshake_carries_no_security_headers() {
    let response = send(handshake(true), Method::GET, "/ws").await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(response.headers().get(header::X_FRAME_OPTIONS), None);
    assert_eq!(
        response.headers().get(header::CONTENT_SECURITY_POLICY),
        None
    );
    assert_eq!(response.headers().get(header::X_CONTENT_TYPE_OPTIONS), None);
    assert_eq!(
        response.headers().get(header::STRICT_TRANSPORT_SECURITY),
        None
    );
}

struct PendingInner;

impl Service<Request<Body>> for PendingInner {
    type Response = Response;
    type Error = Infallible;
    type Future = Ready<Result<Response, Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Pending
    }

    fn call(&mut self, _request: Request<Body>) -> Self::Future {
        ready(Ok(Response::new(Body::empty())))
    }
}

#[test]
fn poll_ready_delegates_to_the_inner_service() {
    let mut service = SecurityHeadersLayer { enable_hsts: false }.layer(PendingInner);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(service.poll_ready(&mut cx).is_pending());
}
