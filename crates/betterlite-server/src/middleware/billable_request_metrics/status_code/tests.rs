#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use betterlite_billing::billable_category::BillableCategory;
use rstest::rstest;
use tower::ServiceExt as _;

use super::super::BillableRequestMetrics;
use super::super::tests::fakes::{MockRecorder, MockSink, app, recorder_factory, sink_factory};

#[rstest]
#[case::ok(StatusCode::OK, true)]
#[case::created(StatusCode::CREATED, true)]
#[case::no_content(StatusCode::NO_CONTENT, true)]
#[case::multiple_choices(StatusCode::MULTIPLE_CHOICES, false)]
#[case::bad_request(StatusCode::BAD_REQUEST, false)]
#[case::not_found(StatusCode::NOT_FOUND, false)]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR, false)]
#[tokio::test]
async fn recorder_runs_only_for_2xx_status_codes(
    #[case] status: StatusCode,
    #[case] records: bool,
) {
    let sink = MockSink::new(false);
    let recorder = MockRecorder::new(false);
    let state = BillableRequestMetrics::new(
        Some(sink_factory(Arc::clone(&sink))),
        Some(recorder_factory(Arc::clone(&recorder))),
    );
    let router = app(state, status, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(sink.calls().len(), 1);
    assert_eq!(usize::from(records), recorder.calls().len());
}

#[tokio::test]
async fn records_the_llm_category_route_and_status_but_skips_the_recorder_on_failure() {
    let sink = MockSink::new(false);
    let recorder = MockRecorder::new(false);
    let state = BillableRequestMetrics::new(
        Some(sink_factory(Arc::clone(&sink))),
        Some(recorder_factory(Arc::clone(&recorder))),
    );
    let router = app(state, StatusCode::NOT_FOUND, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(
        sink.calls(),
        vec![(
            BillableCategory::Llm,
            "/chat/completions".to_owned(),
            StatusCode::NOT_FOUND
        )]
    );
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn records_the_mcp_category_and_route() {
    let sink = MockSink::new(false);
    let state = BillableRequestMetrics::new(Some(sink_factory(Arc::clone(&sink))), None);
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/mcp/tools/list")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(
        sink.calls(),
        vec![(BillableCategory::Mcp, "/mcp".to_owned(), StatusCode::OK)]
    );
}

#[tokio::test]
async fn records_the_a2a_category_and_route() {
    let sink = MockSink::new(false);
    let state = BillableRequestMetrics::new(Some(sink_factory(Arc::clone(&sink))), None);
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/v1/a2a/agent-1/message/send")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(
        sink.calls(),
        vec![(BillableCategory::A2a, "/a2a".to_owned(), StatusCode::OK)]
    );
}

#[tokio::test]
async fn recorder_still_runs_when_the_sink_fails() {
    let sink = MockSink::new(true);
    let recorder = MockRecorder::new(false);
    let state = BillableRequestMetrics::new(
        Some(sink_factory(sink)),
        Some(recorder_factory(Arc::clone(&recorder))),
    );
    let router = app(state, StatusCode::OK, None);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    assert_eq!(recorder.calls().len(), 1);
}

#[rstest]
#[case::with_model_id(Some("model-abc"), Some("model-abc"))]
#[case::without_model_id(None, None)]
#[tokio::test]
async fn forwards_the_model_id_header_to_the_recorder(
    #[case] header: Option<&'static str>,
    #[case] expected: Option<&str>,
) {
    let recorder = MockRecorder::new(false);
    let state = BillableRequestMetrics::new(None, Some(recorder_factory(Arc::clone(&recorder))));
    let router = app(state, StatusCode::OK, header);

    let request = Request::post("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    router.oneshot(request).await.unwrap();

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].3.as_deref(), expected);
}
