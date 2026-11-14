#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;

use crate::failure_hook::ports::BudgetAlertKind;

use super::support::{FakeCache, FakeSender, call_info, decimal, service};

#[tokio::test]
async fn no_destinations_configured_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alerting = None;
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn a_missing_budget_alert_type_sends_nothing() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alert_types = vec![AlertType::FailedTrackingSpend];
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
}

#[tokio::test]
async fn no_configured_budget_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let info = call_info(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn more_than_fifteen_percent_left_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("10"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn an_empty_destination_list_still_reaches_the_sender() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender.clone());
    alerts.alerting = Some(Vec::new());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    assert_eq!(sender.alerts.entries().len(), 1);
}
