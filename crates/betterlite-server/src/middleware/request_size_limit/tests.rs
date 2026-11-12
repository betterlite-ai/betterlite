#![cfg(test)]

use std::convert::Infallible;
use std::future::{Ready, ready};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use futures_util::stream;
use rstest::rstest;
use tower::{Layer as _, Service, ServiceExt as _};

use betterlite_config::sections::request_size_limit::Settings;

use super::SizeLimitLayer;

fn limit(max_bytes: u64, max_mb: f64) -> SizeLimitLayer {
    SizeLimitLayer { max_bytes, max_mb }
}

fn settings(max_request_size_mb: Option<f64>) -> Settings {
    Settings {
        max_request_size_mb,
    }
}

fn with_handler(handler: Router, max_bytes: u64, max_mb: f64) -> Router {
    handler.layer(limit(max_bytes, max_mb))
}

fn echoing(ran: &Arc<AtomicBool>) -> Router {
    let flag = Arc::clone(ran);
    Router::new().fallback(move |body: Bytes| {
        let local = Arc::clone(&flag);
        async move {
            local.store(true, Ordering::Release);
            body.len().to_string()
        }
    })
}

fn chunked(chunks: &[&[u8]]) -> Body {
    let frames = chunks
        .iter()
        .map(|chunk| Ok::<Bytes, io::Error>(Bytes::copy_from_slice(chunk)))
        .collect::<Vec<_>>();
    Body::from_stream(stream::iter(frames))
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body.to_vec())
}

fn body_request(length: Option<&str>, body: Body) -> Request<Body> {
    let mut builder = Request::post("/");
    if let Some(value) = length {
        builder = builder.header(header::CONTENT_LENGTH, value);
    }
    builder.body(body).unwrap()
}

#[rstest]
#[case::unset(None)]
#[case::zero(Some(0.0_f64))]
#[case::negative(Some(-1.0_f64))]
fn a_non_positive_setting_installs_no_layer(#[case] value: Option<f64>) {
    assert!(SizeLimitLayer::from_settings(&settings(value)).is_none());
}

#[rstest]
#[case::whole(5.0_f64, 5_242_880)]
#[case::fraction(0.5_f64, 512 * 1024)]
#[case::below_one_byte(0.000_001_f64, 1)]
#[case::huge(1.0e30_f64, u64::MAX)]
fn a_positive_setting_is_counted_in_bytes(#[case] max_mb: f64, #[case] bytes: u64) {
    let layer = SizeLimitLayer::from_settings(&settings(Some(max_mb))).unwrap();
    assert_eq!(layer.max_bytes, bytes);
}

#[tokio::test]
async fn a_content_length_over_the_limit_is_rejected_without_running_the_handler() {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 10, 5.0);
    let (status, headers, body) =
        send(app, body_request(Some("11"), Body::from("a".repeat(11)))).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes),
        Some(&b"application/json"[..])
    );
    assert_eq!(
        body,
        br#"{"error":"Request size is too large. Max size is 5 MB"}"#
    );
    assert!(!ran.load(Ordering::Acquire), "the handler must not run");
}

#[rstest]
#[case::under("9", 9)]
#[case::equal("10", 10)]
#[tokio::test]
async fn a_content_length_within_the_limit_passes(#[case] length: &str, #[case] size: usize) {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 10, 5.0);
    let (status, _headers, body) = send(
        app,
        body_request(Some(length), Body::from("a".repeat(size))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, size.to_string().as_bytes());
    assert!(ran.load(Ordering::Acquire));
}

#[rstest]
#[case::plain("11", StatusCode::PAYLOAD_TOO_LARGE)]
#[case::space_prefixed(" 5", StatusCode::OK)]
#[case::plus_signed("+5", StatusCode::OK)]
#[case::decimal("5.0", StatusCode::OK)]
#[case::trailing_text("5x", StatusCode::OK)]
#[case::empty("", StatusCode::OK)]
#[case::huge(
    "99999999999999999999999999999999999999999999999999999999",
    StatusCode::PAYLOAD_TOO_LARGE
)]
#[tokio::test]
async fn only_a_plain_decimal_content_length_is_read(
    #[case] length: &str,
    #[case] expected: StatusCode,
) {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 4, 1.0);
    let (status, _headers, _body) = send(app, body_request(Some(length), Body::from("abc"))).await;
    assert_eq!(status, expected);
    assert_eq!(ran.load(Ordering::Acquire), expected == StatusCode::OK);
}

#[rstest]
#[case::last_wins_over("1", "12", StatusCode::PAYLOAD_TOO_LARGE)]
#[case::last_fits("12", "1", StatusCode::OK)]
#[tokio::test]
async fn the_last_duplicate_content_length_wins(
    #[case] first: &str,
    #[case] second: &str,
    #[case] expected: StatusCode,
) {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 4, 1.0);
    let request = Request::post("/")
        .header(header::CONTENT_LENGTH, first)
        .header(header::CONTENT_LENGTH, second)
        .body(Body::from("abc"))
        .unwrap();
    let (status, _headers, _body) = send(app, request).await;
    assert_eq!(status, expected);
    assert_eq!(ran.load(Ordering::Acquire), expected == StatusCode::OK);
}

#[tokio::test]
async fn a_chunked_body_over_the_limit_is_rejected_with_the_same_answer() {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 10, 5.0);
    let payload = chunked(&[b"aaaa", b"aaaa", b"aaaa"]);
    let (status, headers, answered) = send(app, body_request(None, payload)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes),
        Some(&b"application/json"[..])
    );
    assert_eq!(
        answered,
        br#"{"error":"Request size is too large. Max size is 5 MB"}"#
    );
    assert!(!ran.load(Ordering::Acquire), "the handler must not run");
}

#[tokio::test]
async fn a_chunked_body_at_the_limit_passes() {
    let ran = Arc::new(AtomicBool::new(false));
    let app = with_handler(echoing(&ran), 10, 5.0);
    let payload = chunked(&[b"aaa", b"aaa", b"aaaa"]);
    let (status, _headers, answered) = send(app, body_request(None, payload)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answered, b"10");
    assert!(ran.load(Ordering::Acquire));
}

#[tokio::test]
async fn the_error_body_prints_a_fractional_size_without_a_trailing_zero() {
    let response = super::too_large(0.5);
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        body.as_ref(),
        br#"{"error":"Request size is too large. Max size is 0.5 MB"}"#
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
    let mut service = limit(1, 1.0).layer(PendingInner);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(service.poll_ready(&mut cx).is_pending());
}
