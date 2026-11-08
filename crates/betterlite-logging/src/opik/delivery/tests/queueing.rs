use std::time::Duration;

use betterlite_models::decisions::Json;
use tokio::task::yield_now;
use tokio::time::advance;
use tokio_util::task::TaskTracker;

use super::support::{
    Fixture, SPAN_URL, TRACE_URL, call, capture_logs, object, sample_kwargs, sample_response,
};

#[tokio::test]
async fn async_success_queues_events_until_the_batch_size_then_flushes() {
    let (logs, _guard) = capture_logs();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(4);
    let logger = fixture.build(Some(&tracker));
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;
    assert!(fixture.delivery.requests().is_empty());
    assert!(logs.contents().contains("added event to log_queue"));

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;
    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, TRACE_URL);
    assert_eq!(requests[1].url, SPAN_URL);
    assert_eq!(requests[0].batch("traces").len(), 2);
    assert_eq!(requests[1].batch("spans").len(), 2);
}

#[tokio::test]
async fn async_send_batch_sends_traces_before_spans_and_keeps_the_queue() {
    let (logs, _guard) = capture_logs();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1_000);
    let logger = fixture.build(Some(&tracker));
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;
    assert!(fixture.delivery.requests().is_empty());

    logger.async_send_batch().await;
    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
    assert!(logs.contents().contains("Sent traces"));
    assert!(logs.contents().contains("Sent spans"));

    logger.async_send_batch().await;
    assert_eq!(
        fixture.delivery.urls(),
        [TRACE_URL, SPAN_URL, TRACE_URL, SPAN_URL]
    );

    logger.flush_queue().await;
    assert_eq!(
        fixture.delivery.urls(),
        [
            TRACE_URL, SPAN_URL, TRACE_URL, SPAN_URL, TRACE_URL, SPAN_URL
        ]
    );

    logger.async_send_batch().await;
    assert_eq!(fixture.delivery.urls().len(), 6);
}

#[tokio::test]
async fn flush_queue_does_nothing_without_an_async_context_and_sync_still_sends() {
    let (logs, _guard) = capture_logs();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1);
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;
    assert!(fixture.delivery.requests().is_empty());
    assert!(
        logs.contents()
            .contains("Asynchronous processing not initialized")
    );

    logger.log_success_event(call(&kwargs, &response)).await;
    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
}

#[tokio::test]
async fn queue_drops_the_oldest_events_past_max_queue_size() {
    let (logs, _guard) = capture_logs();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1_000);
    fixture.options.max_queue_size = Some(2);
    let logger = fixture.build(Some(&tracker));
    let response = sample_response();
    let first = object(
        r#"{"standard_logging_object": {"messages": [{"role": "user", "content": "first"}], "response": {}, "model": "m"}}"#,
    );
    let second = object(
        r#"{"standard_logging_object": {"messages": [{"role": "user", "content": "second"}], "response": {}, "model": "m"}}"#,
    );

    logger
        .async_log_success_event(call(&first, &response))
        .await;
    logger
        .async_log_success_event(call(&second, &response))
        .await;
    assert!(logs.contents().contains("max_queue_size"));

    logger.async_send_batch().await;
    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 2);
    let traces = requests[0].batch("traces");
    assert_eq!(traces.len(), 1);
    assert_eq!(
        traces[0]["input"],
        serde_json::from_str::<Json>(r#"[{"role": "user", "content": "second"}]"#).unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn periodic_flush_sends_queued_events_after_the_flush_interval() {
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(1_000);
    fixture.options.flush_interval = Some(60);
    let logger = fixture.build(Some(&tracker));
    let kwargs = sample_kwargs();
    let response = sample_response();

    // let the periodic task reach its first sleep before the clock moves
    yield_now().await;
    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;
    assert!(fixture.delivery.requests().is_empty());

    advance(Duration::from_secs(60)).await;
    let mut attempts = 0_u32;
    while fixture.delivery.requests().is_empty() && attempts < 32 {
        yield_now().await;
        attempts = attempts.saturating_add(1);
    }
    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
}
