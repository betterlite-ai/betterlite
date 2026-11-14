#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::budget_email::event::WebhookEventType;

use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType};

use super::BudgetAlertError;
use super::support::{FakeCache, FakeSender, call_info, decimal, service};

fn crossed_user() -> CallInfo {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;
    info
}

#[tokio::test]
async fn the_message_appends_the_user_info_block() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    assert_eq!(
        alert.message,
        "User Budget: Budget Crossed\n Total Budget:`100`\n\n*spend:* `100`\n*max_budget:* `100`\n*user_id:* `user-1`\n*event_group:* `user`\n"
    );
}

#[tokio::test]
async fn the_alert_carries_the_high_level_budget_type_and_event() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let alert = sent.first().expect("one budget alert");
    assert_eq!(alert.level, AlertLevel::High);
    assert_eq!(alert.alert_type, AlertType::BudgetAlerts);
    assert!(alert.metadata.is_empty());
    assert!(alert.kwargs.is_empty());
    let event = alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event, WebhookEventType::BudgetCrossed);
    assert_eq!(event.spend, decimal("100"));
}

#[tokio::test]
async fn a_failed_send_is_reported_and_not_cached() {
    let alerts = service(FakeCache::default(), FakeSender::failing());

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &crossed_user())
        .await;

    assert!(matches!(result, Err(BudgetAlertError::Alert(_))));
    assert!(alerts.cache.writes().is_empty());
}
