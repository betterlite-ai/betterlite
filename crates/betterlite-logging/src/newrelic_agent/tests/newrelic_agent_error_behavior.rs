use super::support::{capture_logs, default_fixture, environment, event, payload};
use crate::callback_logger::CallbackLogger as _;
use crate::newrelic_agent::events::ERROR_METRIC_NAME;

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
    fixture.application.fail_metric(ERROR_METRIC_NAME);
    let ((), logs) = capture_logs(|| logger.log_failure_event(&event(payload())).unwrap());
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
