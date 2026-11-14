#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;

use super::support::{CacheWrite, FakeCache, FakeSender, alerting_args, service};

#[tokio::test]
async fn no_destinations_configured_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alerting = None;

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn a_missing_failed_tracking_type_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alert_types = vec![AlertType::BudgetAlerts];

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn the_model_failure_alert_has_no_user_info() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());

    let result = alerts.failed_tracking_alert("no cost found", "gpt-4").await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one failure alert");
    assert_eq!(sent_alert.message, "Failed Tracking Cost for no cost found");
    assert_eq!(sent_alert.level, AlertLevel::High);
    assert_eq!(sent_alert.alert_type, AlertType::FailedTrackingSpend);
    assert!(sent_alert.user_info.is_none());
    assert!(sent_alert.metadata.is_empty());
    assert!(sent_alert.kwargs.is_empty());
}

#[tokio::test]
async fn the_model_is_cached_for_the_default_ttl() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender);

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(result.is_ok());
    assert_eq!(
        alerts.cache.writes(),
        vec![CacheWrite {
            key: "budget_alerts:failed_tracking:gpt-4".to_owned(),
            value: "SENT".to_owned(),
            ttl_seconds: 86_400,
        }]
    );
}

#[tokio::test]
async fn an_already_sent_model_failure_is_not_reported_again() {
    let sender = FakeSender::default();
    let cache = FakeCache::seeded("budget_alerts:failed_tracking:gpt-4");
    let alerts = service(cache, sender.clone());

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn a_configured_ttl_is_used_for_the_model_cache() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender);
    alerts.alerting_args = alerting_args(120);

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(result.is_ok());
    let writes = alerts.cache.writes();
    let recorded = writes.first().expect("one cache write");
    assert_eq!(recorded.ttl_seconds, 120);
}

#[tokio::test]
async fn a_failed_send_is_reported_and_not_cached() {
    let alerts = service(FakeCache::default(), FakeSender::failing());

    let result = alerts.failed_tracking_alert("boom", "gpt-4").await;

    assert!(matches!(result, Err(super::BudgetAlertError::Alert(_))));
    assert!(alerts.cache.writes().is_empty());
}
