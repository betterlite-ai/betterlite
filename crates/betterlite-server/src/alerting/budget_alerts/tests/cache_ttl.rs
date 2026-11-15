#![cfg(test)]

use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType};

use super::support::{FakeCache, FakeSender, alerting_args, call_info, decimal, service};

fn crossed_user() -> CallInfo {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;
    info
}

#[tokio::test]
async fn the_default_ttl_is_one_day() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender);

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    result.unwrap();
    let writes = alerts.cache.writes();
    let recorded = writes.first().expect("one cache write");
    assert_eq!(recorded.ttl_seconds, 86_400);
    assert_eq!(recorded.value, "SENT");
}

#[tokio::test]
async fn a_configured_ttl_is_used() {
    let sender = FakeSender::default();
    let mut alerts = service(FakeCache::default(), sender);
    alerts.alerting_args = alerting_args(120);

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    result.unwrap();
    let writes = alerts.cache.writes();
    let recorded = writes.first().expect("one cache write");
    assert_eq!(recorded.ttl_seconds, 120);
}

#[tokio::test]
async fn an_existing_key_suppresses_the_alert() {
    let sender = FakeSender::default();
    let cache = FakeCache::seeded("budget_alerts:budget_crossed:user-1");
    let alerts = service(cache, sender.clone());

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    result.unwrap();
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn a_second_call_in_the_window_sends_once() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let info = crossed_user();

    let first = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;
    let second = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    first.unwrap();
    second.unwrap();
    assert_eq!(sender.alerts.entries().len(), 1);
    assert_eq!(alerts.cache.writes().len(), 1);
}
