#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_billing::billable_category::BillableCategory;
use tower::ServiceExt as _;

use super::super::BillableRequestMetrics;
use super::super::tests::fakes::{MockRecorder, MockSink, app, recorder_factory, sink_factory};

#[tokio::test]
async fn passes_through_untouched_when_neither_sink_nor_recorder_is_configured() {
    let state = BillableRequestMetrics::new(None, None);
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn still_classifies_when_only_the_sink_is_configured() {
    let sink = MockSink::new(false);
    let state = BillableRequestMetrics::new(Some(sink_factory(Arc::clone(&sink))), None);
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(
        sink.calls(),
        vec![(
            BillableCategory::Llm,
            "/chat/completions".to_owned(),
            StatusCode::OK
        )]
    );
}

#[tokio::test]
async fn still_classifies_when_only_the_recorder_is_configured() {
    let recorder = MockRecorder::new(false);
    let state = BillableRequestMetrics::new(None, Some(recorder_factory(Arc::clone(&recorder))));
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(
        recorder.calls(),
        vec![(
            BillableCategory::Llm,
            "/chat/completions".to_owned(),
            StatusCode::OK,
            None
        )]
    );
}
