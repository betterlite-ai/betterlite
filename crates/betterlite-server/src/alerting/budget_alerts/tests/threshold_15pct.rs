#![cfg(test)]

use betterlite_logging::budget_email::event::WebhookEventType;

use crate::failure_hook::ports::{BudgetAlertKind, EntityType};

use super::support::{FakeCache, FakeSender, call_info, decimal, service};

#[tokio::test]
async fn ten_percent_left_is_a_fifteen_percent_alert() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("90"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    result.unwrap();
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one threshold alert");
    assert_eq!(
        sent_alert.message,
        "User Budget: 15% or less of budget remaining\n\n*spend:* `90`\n*max_budget:* `100`\n*user_id:* `user-1`\n*event_group:* `user`\n"
    );
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::ThresholdCrossed);
}

#[tokio::test]
async fn exactly_fifteen_percent_left_is_a_fifteen_percent_alert() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("85"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    result.unwrap();
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one threshold alert");
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(
        event.event_message,
        "User Budget: 15% or less of budget remaining"
    );
}

#[tokio::test]
async fn just_over_fifteen_percent_left_sends_nothing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("84.99"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    result.unwrap();
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes().is_empty());
}
