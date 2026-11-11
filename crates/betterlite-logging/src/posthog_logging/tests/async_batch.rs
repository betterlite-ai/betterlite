use std::sync::Arc;

use crate::callback_logger::CallbackLogger as _;
use crate::posthog_logging::logger::PostHogLogger;
use crate::posthog_logging::tests::support::{
    Reply, ScriptedTransport, default_settings, dynamic, event_with, field, fixed_id, list_len,
    logger_with, payload, stop, text,
};

async fn queue_two_credentials(logger: &PostHogLogger) {
    let first = event_with(
        payload(),
        dynamic(&[
            ("posthog_api_key", "first-key"),
            ("posthog_api_url", "https://first.example/"),
        ]),
    );
    let second = event_with(
        payload(),
        dynamic(&[
            ("posthog_api_key", "second-key"),
            ("posthog_api_url", "https://second.example"),
        ]),
    );
    logger
        .async_log_success_event(Arc::new(first.clone()))
        .await
        .expect("the first event is queued");
    logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the second event is queued");
    logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the third event is queued");
}

#[tokio::test]
async fn posthog_async_batch_groups_by_credentials() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    queue_two_credentials(&logger).await;
    logger.flush_queue().await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, "https://first.example/batch/");
    assert_eq!(field(&requests[0].body, "api_key"), &text("first-key"));
    assert_eq!(list_len(field(&requests[0].body, "batch")), 2);
    assert_eq!(requests[1].url, "https://second.example/batch/");
    assert_eq!(field(&requests[1].body, "api_key"), &text("second-key"));
    assert_eq!(list_len(field(&requests[1].body, "batch")), 1);
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_batch_stops_after_a_failed_group() {
    let transport = ScriptedTransport::new(&[Reply::status(400)]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    queue_two_credentials(&logger).await;
    logger.flush_queue().await;
    assert_eq!(transport.urls(), vec!["https://first.example/batch/"]);
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_batch_stops_after_a_transport_failure() {
    let transport = ScriptedTransport::new(&[Reply::network()]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    queue_two_credentials(&logger).await;
    logger.flush_queue().await;
    assert_eq!(transport.urls(), vec!["https://first.example/batch/"]);
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_batch_clears_the_queue_after_a_successful_send() {
    let transport = ScriptedTransport::new(&[Reply::status(200), Reply::status(200)]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    queue_two_credentials(&logger).await;
    logger.flush_queue().await;
    logger.flush_queue().await;
    assert_eq!(transport.urls().len(), 2);
    assert_eq!(logger.sink.queued_depth(), 0);
    stop(&logger, &tasks).await;
}

#[tokio::test]
async fn posthog_async_batch_keeps_one_url_in_separate_groups() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let first = event_with(
        payload(),
        dynamic(&[
            ("posthog_api_key", "first-key"),
            ("posthog_api_url", "https://shared.example"),
        ]),
    );
    let second = event_with(
        payload(),
        dynamic(&[
            ("posthog_api_key", "second-key"),
            ("posthog_api_url", "https://shared.example"),
        ]),
    );
    logger
        .async_log_success_event(Arc::new(first))
        .await
        .expect("the first event is queued");
    logger
        .async_log_success_event(Arc::new(second))
        .await
        .expect("the second event is queued");
    logger.flush_queue().await;
    assert_eq!(transport.urls().len(), 2);
    stop(&logger, &tasks).await;
}
