use betterlite_models::decisions::Json;
use tokio_util::task::TaskTracker;

use super::support::{
    Fixture, SPAN_URL, TRACE_URL, call, capture_logs, sample_kwargs, sample_response,
};

#[tokio::test]
async fn success_events_skip_when_standard_logging_object_is_missing() {
    let (logs, _guard) = capture_logs();
    let mut kwargs = sample_kwargs();
    kwargs.shift_remove("standard_logging_object");
    let response = sample_response();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1);
    let logger = fixture.build(Some(&tracker));

    logger.log_success_event(call(&kwargs, &response)).await;
    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;

    assert!(fixture.delivery.requests().is_empty());
    assert!(logs.contents().contains("no standard_logging_object found"));
}

#[tokio::test]
async fn success_events_skip_when_standard_logging_object_is_null() {
    let mut kwargs = sample_kwargs();
    kwargs.insert("standard_logging_object".to_owned(), Json::Null);
    let response = sample_response();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1);
    let logger = fixture.build(Some(&tracker));

    logger.log_success_event(call(&kwargs, &response)).await;
    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;

    assert!(fixture.delivery.requests().is_empty());
}

#[tokio::test]
async fn success_events_process_any_non_null_standard_logging_object() {
    let mut kwargs = sample_kwargs();
    kwargs.insert(
        "standard_logging_object".to_owned(),
        Json::Text("not an object".to_owned()),
    );
    let response = sample_response();
    let fixture = Fixture::new();
    let logger = fixture.build(None);

    logger.log_success_event(call(&kwargs, &response)).await;

    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
}
