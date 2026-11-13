use crate::callback_logger::CallbackLogger as _;
use crate::datadog::metrics::datadog_metrics::DatadogMetricsLogger;
use crate::datadog::metrics::series::REQUEST_COUNT_METRIC;

use super::support;

fn status_tags(logger: &DatadogMetricsLogger) -> Vec<String> {
    let queued = logger.queue.pending();
    let count = queued
        .iter()
        .find(|series| series.metric == REQUEST_COUNT_METRIC)
        .expect("the request count series is queued");
    count.tags.clone()
}

#[tokio::test]
async fn status_success_event_uses_200() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_success_event(support::event(support::payload()))
        .await
        .expect("the success event records");
    assert!(status_tags(&logger).contains(&"status_code:200".to_owned()));
}

#[tokio::test]
async fn status_failure_event_uses_the_error_code() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::failure_payload(Some("429"))))
        .await
        .expect("the failure event records");
    assert!(status_tags(&logger).contains(&"status_code:429".to_owned()));
}

#[tokio::test]
async fn status_failure_event_defaults_to_500_without_error_information() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::failure_without_information()))
        .await
        .expect("the failure event records");
    assert!(status_tags(&logger).contains(&"status_code:500".to_owned()));
}

#[tokio::test]
async fn status_failure_event_defaults_to_500_for_a_null_error_code() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::failure_payload(None)))
        .await
        .expect("the failure event records");
    assert!(status_tags(&logger).contains(&"status_code:500".to_owned()));
}

#[tokio::test]
async fn status_failure_event_keeps_an_empty_error_code() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        support::MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::failure_payload(Some(""))))
        .await
        .expect("the failure event records");
    assert!(status_tags(&logger).contains(&"status_code:".to_owned()));
}
