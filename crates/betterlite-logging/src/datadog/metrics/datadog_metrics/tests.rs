#![cfg(test)]

mod clock;
mod flush;
mod health;
mod latency;
mod overhead_tag_exclusion;
mod status;
mod support;
mod tags;
mod upload;

use crate::callback_logger::CallbackLogger as _;
use crate::datadog::metrics::series::{
    DatadogMetricPoint, PROVIDER_LATENCY_METRIC, REQUEST_COUNT_METRIC, TOTAL_LATENCY_METRIC,
};

use self::support::{MetricsInit, Outcome};

#[tokio::test]
async fn callback_logger_name_is_datadog_metrics() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    assert_eq!(logger.callback_name(), "datadog_metrics");
}

#[tokio::test]
async fn callback_logger_records_the_success_metrics() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_success_event(support::event(support::payload()))
        .await
        .expect("the success event records");
    let queued = logger.queue.pending();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].metric, TOTAL_LATENCY_METRIC);
    assert_eq!(
        queued[0].points,
        vec![DatadogMetricPoint {
            timestamp: 1_704_067_202,
            value: 2.5,
        }]
    );
    assert_eq!(queued[1].metric, REQUEST_COUNT_METRIC);
    assert!(queued[1].tags.contains(&"status_code:200".to_owned()));
}

#[tokio::test]
async fn callback_logger_omits_provider_latency_without_a_provider_start() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_success_event(support::event(support::payload()))
        .await
        .expect("the success event records");
    let queued = logger.queue.pending();
    assert!(
        !queued
            .iter()
            .any(|series| series.metric == PROVIDER_LATENCY_METRIC)
    );
}

#[tokio::test]
async fn callback_logger_failure_event_records_the_error_status() {
    let (logger, _transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    logger
        .async_log_failure_event(support::event(support::failure_payload(Some("429"))))
        .await
        .expect("the failure event records");
    let queued = logger.queue.pending();
    let count = queued
        .iter()
        .find(|series| series.metric == REQUEST_COUNT_METRIC)
        .expect("the request count series is queued");
    assert!(count.tags.contains(&"status_code:429".to_owned()));
}

#[tokio::test]
async fn callback_logger_flushes_at_the_batch_size() {
    let (logger, transport) = support::build(
        &support::environment(&[("DD_API_KEY", "secret-key")]),
        MetricsInit {
            batch_size: Some(2),
        },
        vec![Outcome::Reply(202)],
    );
    logger
        .async_log_success_event(support::event(support::payload()))
        .await
        .expect("the success event records");
    assert_eq!(transport.call_count(), 1);
    assert!(logger.queue.pending().is_empty());
}
