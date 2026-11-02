#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::runtime::Runtime;
use tower::ServiceExt as _;

use super::super::BillableRequestMetrics;
use super::super::tests::fakes::{
    MockRecorder, MockSink, app, logged, recorder_factory, sink_factory,
};

#[test]
fn sink_failure_logs_a_warning_and_does_not_block_the_response() {
    let sink = MockSink::new(true);
    let state = BillableRequestMetrics::new(Some(sink_factory(sink)), None);
    let router = app(state, StatusCode::OK, None);

    let (status, log) = logged(|| {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async {
            let request = Request::post("/v1/chat/completions")
                .body(Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            response.status()
        })
    });

    assert_eq!(status, StatusCode::OK);
    assert!(log.contains("gateway request metering failed"));
}

#[test]
fn recorder_failure_logs_a_warning_and_does_not_block_the_response() {
    let recorder = MockRecorder::new(true);
    let state = BillableRequestMetrics::new(None, Some(recorder_factory(recorder)));
    let router = app(state, StatusCode::OK, None);

    let (status, log) = logged(|| {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async {
            let request = Request::post("/v1/chat/completions")
                .body(Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            response.status()
        })
    });

    assert_eq!(status, StatusCode::OK);
    assert!(log.contains("billable request metering failed"));
}
