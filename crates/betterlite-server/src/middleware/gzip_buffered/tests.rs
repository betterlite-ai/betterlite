#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read as _};
use std::path::PathBuf;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use axum::response::Response;
use flate2::read::GzDecoder;
use futures_util::stream;
use rstest::rstest;
use tower::ServiceExt as _;

use super::{GzipBufferedLayer, accepts_gzip};

fn gunzip(encoded: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(encoded).read_to_end(&mut decoded).unwrap();
    decoded
}

fn text_app(size: usize) -> Router {
    Router::new()
        .fallback(move || async move { "a".repeat(size) })
        .layer(GzipBufferedLayer)
}

fn status_app(
    status: StatusCode,
    extra_headers: Vec<(&'static str, &'static str)>,
    size: usize,
) -> Router {
    Router::new()
        .fallback(move || {
            let owned_headers = extra_headers.clone();
            async move {
                let mut response = Response::new(Body::from("a".repeat(size)));
                *response.status_mut() = status;
                for (name, value) in owned_headers {
                    response.headers_mut().insert(
                        HeaderName::from_static(name),
                        HeaderValue::from_static(value),
                    );
                }
                response
            }
        })
        .layer(GzipBufferedLayer)
}

fn streaming_app() -> Router {
    Router::new()
        .fallback(|| async {
            let frames = vec![
                Ok::<Bytes, io::Error>(Bytes::from("a".repeat(400))),
                Ok(Bytes::from("a".repeat(400))),
            ];
            Response::new(Body::from_stream(stream::iter(frames)))
        })
        .layer(GzipBufferedLayer)
}

async fn send(app: Router, method: Method, accept: Option<&str>) -> Response {
    let mut builder = Request::builder().method(method).uri("/");
    if let Some(value) = accept {
        builder = builder.header(header::ACCEPT_ENCODING, value);
    }
    app.oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body_bytes(response: Response) -> Bytes {
    to_bytes(response.into_body(), usize::MAX).await.unwrap()
}

fn header_value(headers: &HeaderMap, name: HeaderName) -> Option<&[u8]> {
    headers.get(name).map(HeaderValue::as_bytes)
}

#[rstest]
#[case::plain("gzip", true)]
#[case::uppercase("GZIP", true)]
#[case::spaced(" gzip ; q=0.5 ", true)]
#[case::zero_weight("gzip;q=0", false)]
#[case::zero_decimal("gzip;q=0.0", false)]
#[case::half("gzip;q=0.5", true)]
#[case::extra_parameter("gzip;level=9", true)]
#[case::parameter_then_weight("gzip;level=9;q=0", false)]
#[case::uppercase_parameter("gzip;Q=0", false)]
#[case::x_gzip("x-gzip", true)]
#[case::x_gzip_zero("x-gzip;q=0", false)]
#[case::any("*", true)]
#[case::any_zero("*;q=0", false)]
#[case::gzip_beats_star("gzip, *;q=0", true)]
#[case::gzip_after_star("*;q=0, gzip", true)]
#[case::bad_weight("gzip;q=abc", false)]
#[case::bad_weight_falls_to_star("gzip;q=abc, *", true)]
#[case::duplicate_last_wins("gzip;q=0, gzip", true)]
#[case::duplicate_last_zero("gzip, gzip;q=0", false)]
#[case::gzip_zero_beats_x_gzip("x-gzip, gzip;q=0", false)]
#[case::empty_parts(", ,", false)]
#[case::empty("", false)]
#[case::identity("identity", false)]
#[case::other_codings("br, deflate", false)]
#[case::above_one("gzip;q=2", true)]
#[case::negative("gzip;q=-1", false)]
fn accepts_gzip_follows_the_q_values(#[case] header: &str, #[case] expected: bool) {
    assert_eq!(accepts_gzip(header.as_bytes()), expected);
}

#[rstest]
#[case::invalid_utf8_alone(b"\xff", false)]
#[case::invalid_utf8_parameter(b"gzip;\xff", true)]
fn accepts_gzip_reads_the_header_one_byte_per_character(
    #[case] raw: &[u8],
    #[case] expected: bool,
) {
    assert_eq!(accepts_gzip(raw), expected);
}

#[rstest]
fn replays_the_accept_encoding_corpus(#[files("fuzz/corpus/accept_encoding/*")] path: PathBuf) {
    let raw = fs::read(&path).unwrap();
    let accepted = accepts_gzip(&raw);
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return;
    };
    if name.starts_with("accepted-") {
        assert!(accepted, "{name}");
    }
    if name.starts_with("refused-") {
        assert!(!accepted, "{name}");
    }
}

#[tokio::test]
async fn gzip_compresses_a_body_at_the_minimum_size() {
    let response = send(text_app(500), Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        Some(&b"gzip"[..])
    );
    assert_eq!(
        header_value(response.headers(), header::VARY),
        Some(&b"Accept-Encoding"[..])
    );
    let declared = header_value(response.headers(), header::CONTENT_LENGTH).map(<[u8]>::to_vec);
    let encoded = body_bytes(response).await;
    assert_eq!(declared.as_deref(), Some(&*encoded));
    assert_eq!(gunzip(&encoded), "a".repeat(500).into_bytes());
}

#[tokio::test]
async fn gzip_leaves_a_body_under_the_minimum_alone() {
    let response = send(text_app(499), Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(header_value(response.headers(), header::VARY), None);
    assert_eq!(body_bytes(response).await.len(), 499);
}

#[rstest]
#[case::refused(Some("identity"))]
#[case::absent(None)]
#[tokio::test]
async fn gzip_adds_vary_even_when_the_client_does_not_accept_it(#[case] accept: Option<&str>) {
    let response = send(text_app(500), Method::GET, accept).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(
        header_value(response.headers(), header::VARY),
        Some(&b"Accept-Encoding"[..])
    );
    assert_eq!(body_bytes(response).await.len(), 500);
}

#[tokio::test]
async fn gzip_appends_to_an_existing_vary() {
    let app = status_app(StatusCode::OK, vec![("vary", "Origin")], 500);
    let response = send(app, Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::VARY),
        Some(&b"Origin, Accept-Encoding"[..])
    );
}

#[tokio::test]
async fn gzip_does_not_deduplicate_vary() {
    let app = status_app(StatusCode::OK, vec![("vary", "Accept-Encoding")], 500);
    let response = send(app, Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::VARY),
        Some(&b"Accept-Encoding, Accept-Encoding"[..])
    );
}

#[rstest]
#[case::partial_content(StatusCode::PARTIAL_CONTENT)]
#[case::switching_protocols(StatusCode::SWITCHING_PROTOCOLS)]
#[tokio::test]
async fn gzip_skips_statuses_it_never_touches(#[case] status: StatusCode) {
    let response = send(status_app(status, vec![], 600), Method::GET, Some("gzip")).await;
    assert_eq!(response.status(), status);
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(header_value(response.headers(), header::VARY), None);
}

#[rstest]
#[case::etag("etag", "\"abc\"")]
#[case::content_encoding("content-encoding", "br")]
#[case::no_transform("cache-control", "no-transform")]
#[case::no_transform_in_a_list("cache-control", "max-age=0, No-Transform")]
#[tokio::test]
async fn gzip_skips_responses_it_must_not_reencode(
    #[case] name: &'static str,
    #[case] value: &'static str,
) {
    let app = status_app(StatusCode::OK, vec![(name, value)], 600);
    let response = send(app, Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(header_value(response.headers(), header::VARY), None);
    assert_eq!(body_bytes(response).await.len(), 600);
}

#[tokio::test]
async fn gzip_passes_a_stream_untouched() {
    let response = send(streaming_app(), Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(header_value(response.headers(), header::VARY), None);
    assert_eq!(body_bytes(response).await.len(), 800);
}

#[tokio::test]
async fn gzip_compresses_a_body_at_the_off_loop_threshold() {
    let size = 1024 * 1024;
    let response = send(text_app(size), Method::GET, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        Some(&b"gzip"[..])
    );
    assert_eq!(
        gunzip(&body_bytes(response).await),
        "a".repeat(size).into_bytes()
    );
}

#[tokio::test]
async fn gzip_passes_a_head_response_untouched() {
    let response = send(text_app(600), Method::HEAD, Some("gzip")).await;
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
    assert_eq!(header_value(response.headers(), header::VARY), None);
    assert_eq!(body_bytes(response).await.len(), 600);
}

#[tokio::test]
async fn gzip_reads_only_the_first_accept_encoding_header() {
    let request = Request::builder()
        .uri("/")
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(Body::empty())
        .unwrap();
    let response = text_app(600).oneshot(request).await.unwrap();
    assert_eq!(
        header_value(response.headers(), header::CONTENT_ENCODING),
        None
    );
}
