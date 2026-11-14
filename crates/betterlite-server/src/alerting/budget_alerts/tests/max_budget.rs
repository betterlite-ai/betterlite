#![cfg(test)]

use betterlite_logging::budget_email::event::WebhookEventType;

use crate::failure_hook::ports::{BudgetAlertKind, EntityType};

use super::support::{FakeCache, FakeSender, call_info, decimal, service};

#[tokio::test]
async fn spend_equal_to_the_budget_crosses_it() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    assert_eq!(
        alert.message,
        "User Budget: Budget Crossed\n Total Budget:`100`\n\n*spend:* `100`\n*max_budget:* `100`\n*user_id:* `user-1`\n*event_group:* `user`\n"
    );
    let event = alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::BudgetCrossed);
}

#[tokio::test]
async fn spend_above_the_budget_crosses_it() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("150"));
    info.max_budget = Some(decimal("100"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    let event = alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::BudgetCrossed);
    assert_eq!(event.spend, decimal("150"));
}

#[tokio::test]
async fn a_crossed_soft_budget_is_reported_before_the_budget_crossing() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("120"));
    info.max_budget = Some(decimal("100"));
    info.soft_budget = Some(decimal("50"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    assert_eq!(
        alert.message,
        "User Budget: Total Soft Budget:`50`Budget Crossed\n Total Budget:`100`\n\n*spend:* `120`\n*max_budget:* `100`\n*soft_budget:* `50`\n*user_id:* `user-1`\n*event_group:* `user`\n"
    );
    let event = alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::BudgetCrossed);
}

#[tokio::test]
async fn an_uncrossed_soft_budget_leaves_the_budget_message_alone() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.soft_budget = Some(decimal("200"));

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    let event = alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(
        event.event_message,
        "User Budget: Budget Crossed\n Total Budget:`100`"
    );
}
