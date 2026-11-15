#![cfg(test)]

use betterlite_logging::budget_email::event::WebhookEventType;

use crate::failure_hook::ports::BudgetAlertKind;

use super::support::{FakeCache, FakeSender, call_info, decimal, service};

#[tokio::test]
async fn spend_equal_to_the_soft_budget_sends_a_soft_budget_alert() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("50"));
    info.soft_budget = Some(decimal("50"));
    info.token = Some("hashed-key".to_owned());

    let result = alerts
        .budget_alerts(BudgetAlertKind::SoftBudget, &info)
        .await;

    result.unwrap();
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one soft budget alert");
    assert_eq!(
        sent_alert.message,
        "Soft Budget Crossed: Total Soft Budget:`50`\n\n*spend:* `50`\n*soft_budget:* `50`\n*event_group:* `key`\n"
    );
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::SoftBudgetCrossed);
    assert_eq!(
        event.event_message,
        "Soft Budget Crossed: Total Soft Budget:`50`"
    );
    assert_eq!(event.soft_budget, Some(decimal("50")));
}

#[tokio::test]
async fn spend_below_the_soft_budget_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("49.99"));
    info.soft_budget = Some(decimal("50"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::SoftBudget, &info)
        .await;

    result.unwrap();
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}

#[tokio::test]
async fn spend_above_the_soft_budget_sends_a_soft_budget_alert() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("75"));
    info.soft_budget = Some(decimal("50"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::SoftBudget, &info)
        .await;

    result.unwrap();
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one soft budget alert");
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::SoftBudgetCrossed);
    assert_eq!(event.spend, decimal("75"));
}
