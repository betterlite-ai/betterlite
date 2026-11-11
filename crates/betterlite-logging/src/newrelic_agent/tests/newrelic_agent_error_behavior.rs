use super::support::{capture_logs, default_fixture, environment, event, payload};
use crate::callback_logger::CallbackLogger as _;
use crate::newrelic_agent::events::{ERROR_METRIC_NAME, MESSAGE_EVENT_NAME, SUMMARY_EVENT_NAME};

#[test]
fn a_failure_records_only_the_error_metric() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let ((), logs) = capture_logs(|| logger.log_failure_event(&event(payload())).unwrap());
    assert_eq!(
        fixture.application.metrics_named(ERROR_METRIC_NAME),
        vec![1]
    );
    assert!(fixture.application.events().is_empty());
    assert!(!logs.contains("Failed to record New Relic error metric"));
}

#[test]
fn a_failure_emits_the_supportability_metric_only_once_per_window() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let failure = event(payload());
    logger.log_failure_event(&failure).unwrap();
    logger.log_failure_event(&failure).unwrap();
    assert_eq!(
        fixture.application.metrics_named(ERROR_METRIC_NAME),
        vec![1, 1]
    );
    assert_eq!(fixture.application.supportability_metrics().len(), 1);
}

#[test]
fn a_failed_error_metric_counts_a_callback_failure() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let failure = event(payload());
    logger.log_failure_event(&failure).unwrap();
    fixture.application.fail_next_metrics(1);
    let ((), logs) = capture_logs(|| logger.log_failure_event(&failure).unwrap());
    assert_eq!(fixture.counter.calls(), vec!["newrelic"]);
    assert!(logs.contains("Failed to record New Relic error metric"));
}

#[test]
fn a_disabled_integration_records_no_error_metric() {
    let mut fixture = default_fixture();
    fixture.environment = environment(&[]);
    let logger = fixture.build();
    assert!(!logger.enabled);
    logger.log_failure_event(&event(payload())).unwrap();
    assert!(fixture.application.metrics().is_empty());
    assert!(fixture.application.events().is_empty());
}

#[test]
fn a_disabled_application_records_no_error_metric() {
    let fixture = default_fixture();
    fixture.application.set_enabled(false);
    let logger = fixture.build();
    assert!(logger.enabled);
    logger.log_failure_event(&event(payload())).unwrap();
    assert!(fixture.application.metrics().is_empty());
    assert!(fixture.counter.calls().is_empty());
}

#[test]
fn a_failed_message_stops_the_remaining_messages() {
    let fixture = default_fixture();
    fixture.application.fail_event(MESSAGE_EVENT_NAME);
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    assert_eq!(
        fixture.application.events_named(MESSAGE_EVENT_NAME).len(),
        0
    );
    assert_eq!(
        fixture.application.events_named(SUMMARY_EVENT_NAME).len(),
        1
    );
    assert_eq!(fixture.counter.calls(), vec!["newrelic"]);
}

#[test]
fn a_failed_supportability_metric_does_not_count_a_callback_failure() {
    let fixture = default_fixture();
    fixture.application.fail_next_metrics(1);
    let logger = fixture.build();
    let ((), logs) = capture_logs(|| logger.log_success_event(&event(payload())).unwrap());
    assert!(fixture.counter.calls().is_empty());
    assert!(logs.contains("Failed to emit supportability metric"));
}

#[tokio::test]
async fn sync_and_async_failure_events_record_the_same_metric() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger
        .async_log_failure_event(event(payload()))
        .await
        .unwrap();
    logger.log_failure_event(&event(payload())).unwrap();
    assert_eq!(
        fixture.application.metrics_named(ERROR_METRIC_NAME),
        vec![1, 1]
    );
    assert!(fixture.application.events().is_empty());
}
