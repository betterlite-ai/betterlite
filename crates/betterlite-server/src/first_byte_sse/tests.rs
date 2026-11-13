#![cfg(test)]

use std::future::pending;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use futures_util::StreamExt as _;
use futures_util::stream;
use tokio::time::sleep;

use super::{
    HookRaised, LateFailureHook, ProducedResponse, SseFailure, StreamedResponse,
    open_sse_before_first_byte,
};
use crate::keepalive_interval::PingSetting;

fn produced_stream(
    status: StatusCode,
    headers: &[(&str, &str)],
    chunks: Vec<&'static [u8]>,
) -> ProducedResponse {
    let mut response_headers = HeaderMap::new();
    for (name, value) in headers {
        response_headers.insert(
            HeaderName::try_from(*name).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }
    let body = stream::iter(
        chunks
            .into_iter()
            .map(|chunk| Ok(Bytes::from_static(chunk))),
    )
    .boxed();
    ProducedResponse::Streaming(StreamedResponse {
        status,
        headers: response_headers,
        body,
    })
}

async fn body_bytes(response: Response) -> Bytes {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body")
}

fn failure(text: &str) -> SseFailure {
    serde_json::from_str(text).expect("valid failure fixture")
}

async fn late_body(
    outcome: Result<ProducedResponse, SseFailure>,
    hook: Option<Arc<dyn LateFailureHook>>,
) -> Bytes {
    let response = open_sse_before_first_byte(
        async move {
            sleep(Duration::from_millis(1500)).await;
            outcome
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        hook,
    )
    .await
    .expect("late response");
    body_bytes(response).await
}

enum HookAnswer {
    Replace(SseFailure),
    Keep,
    Raise(SseFailure),
    Break(String),
}

struct HookLog {
    received: Mutex<Option<SseFailure>>,
    calls: AtomicUsize,
}

struct FakeHook {
    answer: HookAnswer,
    log: Arc<HookLog>,
}

impl FakeHook {
    fn arc(answer: HookAnswer) -> (Arc<dyn LateFailureHook>, Arc<HookLog>) {
        let log = Arc::new(HookLog {
            received: Mutex::new(None),
            calls: AtomicUsize::new(0),
        });
        let hook = Self {
            answer,
            log: Arc::clone(&log),
        };
        (Arc::new(hook), log)
    }
}

#[async_trait]
impl LateFailureHook for FakeHook {
    async fn late_failure(&self, failure: &SseFailure) -> Result<Option<SseFailure>, HookRaised> {
        self.log.calls.fetch_add(1, Ordering::SeqCst);
        *self.log.received.lock().expect("hook log") = Some(failure.clone());
        match &self.answer {
            HookAnswer::Replace(replacement) => Ok(Some(replacement.clone())),
            HookAnswer::Keep => Ok(None),
            HookAnswer::Raise(replacement) => Err(HookRaised::Replacement(replacement.clone())),
            HookAnswer::Break(message) => Err(HookRaised::Broken(message.clone())),
        }
    }
}

fn assert_hook_saw(log: &HookLog, expected: &SseFailure) {
    assert_eq!(
        log.received.lock().expect("hook log").as_ref(),
        Some(expected)
    );
    assert_eq!(log.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn without_an_interval_the_streamed_result_is_returned_unchanged() {
    let produced = produced_stream(
        StatusCode::CREATED,
        &[("x-upstream", "yes")],
        vec![b"alpha", b"beta"],
    );
    let response = open_sse_before_first_byte(
        async { Ok::<ProducedResponse, SseFailure>(produced) },
        None,
        "text/event-stream",
        None,
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-upstream"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert_eq!(body_bytes(response).await, Bytes::from_static(b"alphabeta"));
}

#[tokio::test]
async fn without_an_interval_the_plain_result_is_returned_unchanged() {
    let response = open_sse_before_first_byte(
        async {
            Ok::<ProducedResponse, SseFailure>(ProducedResponse::Plain(
                Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .header("x-origin", "guardrail")
                    .body(Body::from("refused"))
                    .expect("plain response"),
            ))
        },
        None,
        "text/event-stream",
        None,
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.headers().get("x-origin"),
        Some(&HeaderValue::from_static("guardrail"))
    );
    assert_eq!(body_bytes(response).await, Bytes::from_static(b"refused"));
}

#[tokio::test]
async fn without_an_interval_the_failure_is_returned_unchanged() {
    let failure = failure(r#"{"status_code": 429, "detail": "slow down"}"#);
    let returned = failure.clone();
    let result = open_sse_before_first_byte(
        async move { Err::<ProducedResponse, SseFailure>(returned) },
        None,
        "text/event-stream",
        None,
    )
    .await;
    assert_eq!(result.expect_err("failure"), failure);
}

#[tokio::test(start_paused = true)]
async fn a_result_inside_one_interval_writes_nothing_early() {
    let response = open_sse_before_first_byte(
        async {
            sleep(Duration::from_millis(500)).await;
            Ok::<ProducedResponse, SseFailure>(produced_stream(
                StatusCode::CREATED,
                &[],
                vec![b"alpha"],
            ))
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        None,
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(body_bytes(response).await, Bytes::from_static(b"alpha"));
}

#[tokio::test(start_paused = true)]
async fn a_failure_inside_one_interval_is_returned_unchanged() {
    let failure = failure(r#"{"status_code": 503, "detail": "no capacity"}"#);
    let returned = failure.clone();
    let result = open_sse_before_first_byte(
        async move {
            sleep(Duration::from_millis(500)).await;
            Err::<ProducedResponse, SseFailure>(returned)
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        None,
    )
    .await;
    assert_eq!(result.expect_err("failure"), failure);
}

#[tokio::test(start_paused = true)]
async fn a_plain_result_inside_one_interval_is_returned_unchanged() {
    let response = open_sse_before_first_byte(
        async {
            sleep(Duration::from_millis(500)).await;
            Ok::<ProducedResponse, SseFailure>(ProducedResponse::Plain(
                Response::builder()
                    .status(StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS)
                    .header("x-origin", "guardrail")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"blocked":true}"#))
                    .expect("plain response"),
            ))
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        None,
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);
    assert_eq!(
        response.headers().get("x-origin"),
        Some(&HeaderValue::from_static("guardrail"))
    );
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_eq!(
        body_bytes(response).await,
        Bytes::from_static(br#"{"blocked":true}"#)
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_stream_opens_with_the_keepalive_headers_and_pings() {
    let response = open_sse_before_first_byte(
        async {
            sleep(Duration::from_millis(2500)).await;
            Ok::<ProducedResponse, SseFailure>(produced_stream(
                StatusCode::CREATED,
                &[("x-upstream", "yes")],
                vec![b"alpha", b"beta"],
            ))
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        None,
    )
    .await
    .expect("early response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        ))
    );
    assert_eq!(
        response.headers().get(CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-cache"))
    );
    assert_eq!(
        response.headers().get("x-accel-buffering"),
        Some(&HeaderValue::from_static("no"))
    );
    assert!(response.headers().get("x-upstream").is_none());
    assert_eq!(
        body_bytes(response).await,
        Bytes::from_static(b": ping\n\n: ping\n\nalphabeta")
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_plain_response_is_replayed_as_one_data_frame() {
    let body = late_body(
        Ok(ProducedResponse::Plain(
            Response::builder()
                .status(StatusCode::IM_A_TEAPOT)
                .header("x-ignored", "yes")
                .body(Body::from(r#"{"blocked":true}"#))
                .expect("plain response"),
        )),
        None,
    )
    .await;
    assert_eq!(
        body,
        Bytes::from_static(b": ping\n\ndata: {\"blocked\":true}\n\ndata: [DONE]\n\n")
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_plain_response_with_an_empty_body_writes_an_empty_object() {
    let body = late_body(
        Ok(ProducedResponse::Plain(
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .expect("plain response"),
        )),
        None,
    )
    .await;
    assert_eq!(
        body,
        Bytes::from_static(b": ping\n\ndata: {}\n\ndata: [DONE]\n\n")
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_failure_is_written_as_the_error_and_done_frames() {
    let body = late_body(
        Err(failure(r#"{"status_code": 429, "detail": "slow down"}"#)),
        None,
    )
    .await;
    assert_eq!(
        body,
        Bytes::from_static(
            b": ping\n\ndata: {\"error\":{\"code\":\"429\",\"message\":\"slow down\",\"param\":null,\"type\":\"rate_limit_error\"}}\n\ndata: [DONE]\n\n"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_failure_uses_the_replacement_the_hook_returns() {
    let original = failure(r#"{"status_code": 500, "detail": "the original"}"#);
    let replacement = failure(r#"{"status_code": 401, "detail": "the hook's words"}"#);
    let (hook, log) = FakeHook::arc(HookAnswer::Replace(replacement));
    let body = late_body(Err(original.clone()), Some(hook)).await;
    assert_hook_saw(&log, &original);
    assert_eq!(
        body,
        Bytes::from_static(
            b": ping\n\ndata: {\"error\":{\"code\":\"401\",\"message\":\"the hook's words\",\"param\":null,\"type\":\"authentication_error\"}}\n\ndata: [DONE]\n\n"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_failure_uses_the_replacement_the_hook_raises() {
    let original = failure(r#"{"status_code": 500, "detail": "the original"}"#);
    let replacement = failure(r#"{"status_code": 403, "detail": "raised"}"#);
    let (hook, log) = FakeHook::arc(HookAnswer::Raise(replacement));
    let body = late_body(Err(original.clone()), Some(hook)).await;
    assert_hook_saw(&log, &original);
    assert_eq!(
        body,
        Bytes::from_static(
            b": ping\n\ndata: {\"error\":{\"code\":\"403\",\"message\":\"raised\",\"param\":null,\"type\":\"permission_error\"}}\n\ndata: [DONE]\n\n"
        )
    );
}

#[rstest::rstest]
#[case::keeps_the_original(HookAnswer::Keep)]
#[case::broken_hook_keeps_the_original(HookAnswer::Break(String::from("the callback failed")))]
#[tokio::test(start_paused = true)]
async fn a_late_failure_without_a_usable_replacement_keeps_the_original(
    #[case] answer: HookAnswer,
) {
    let original = failure(r#"{"status_code": 500, "detail": "the original"}"#);
    let (hook, log) = FakeHook::arc(answer);
    let body = late_body(Err(original.clone()), Some(hook)).await;
    assert_hook_saw(&log, &original);
    assert_eq!(
        body,
        Bytes::from_static(
            b": ping\n\ndata: {\"error\":{\"code\":\"500\",\"message\":\"the original\",\"param\":null,\"type\":\"internal_server_error\"}}\n\ndata: [DONE]\n\n"
        )
    );
}

struct DropGuard(Arc<AtomicBool>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn dropping_the_body_drops_the_pending_future() {
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Arc::clone(&dropped);
    let response = open_sse_before_first_byte(
        async move {
            let _guard = DropGuard(guard);
            pending::<Result<ProducedResponse, SseFailure>>().await
        },
        Some(PingSetting::Seconds(1.0)),
        "text/event-stream",
        None,
    )
    .await
    .expect("early response");
    assert!(!dropped.load(Ordering::SeqCst));
    drop(response);
    assert!(dropped.load(Ordering::SeqCst));
}
