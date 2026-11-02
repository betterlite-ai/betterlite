#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
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
