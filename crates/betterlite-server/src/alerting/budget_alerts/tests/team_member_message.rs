#![cfg(test)]

use betterlite_logging::budget_email::event::EntityType as EventEntityType;

use crate::failure_hook::ports::{BudgetAlertKind, EntityType};

use super::support::{FakeCache, FakeSender, call_info, decimal, service};

#[tokio::test]
async fn a_team_member_key_alert_is_labelled_team_member() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.token = Some("hashed-key".to_owned());
    info.user_id = Some("user-1".to_owned());
    info.team_id = Some("team-1".to_owned());
    info.event_group = EntityType::TeamMember;

    let result = alerts
        .budget_alerts(BudgetAlertKind::TokenBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one team member alert");
    assert!(
        sent_alert
            .message
            .starts_with("Team Member Budget: Budget Crossed\n Total Budget:`100`\n\n")
    );
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(event.event_group, EventEntityType::TeamMember);
    assert_eq!(
        event.event_message,
        "Team Member Budget: Budget Crossed\n Total Budget:`100`"
    );
}

#[tokio::test]
async fn the_label_override_applies_to_any_budget_type() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.team_id = Some("team-1".to_owned());
    info.event_group = EntityType::TeamMember;

    let result = alerts
        .budget_alerts(BudgetAlertKind::UserBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one team member alert");
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(
        event.event_message,
        "Team Member Budget: Budget Crossed\n Total Budget:`100`"
    );
}

#[tokio::test]
async fn any_other_group_keeps_the_budget_type_label() {
    let sender = FakeSender::default();
    let alerts = service(FakeCache::default(), sender.clone());
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.token = Some("hashed-key".to_owned());

    let result = alerts
        .budget_alerts(BudgetAlertKind::TokenBudget, &info)
        .await;

    assert!(result.is_ok());
    let sent = sender.alerts.entries();
    let sent_alert = sent.first().expect("one budget alert");
    let event = sent_alert
        .user_info
        .as_ref()
        .expect("the webhook event is attached");
    assert_eq!(
        event.event_message,
        "Key Budget: Budget Crossed\n Total Budget:`100`"
    );
}
