#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_logging::budget_email::event::{EntityType, WebhookEventType};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::failure_hook::ports::{BudgetAlertKind, CallInfo, EntityType as PortEntityType};

use super::events::{budget_alert, budget_webhook_event, entity_type};
use super::support::{call_info, decimal};

fn full_info() -> CallInfo {
    let mut info = call_info(decimal("300"));
    info.max_budget = Some(decimal("1000"));
    info.soft_budget = Some(decimal("500"));
    info.token = Some("hashed-key".to_owned());
    info.customer_id = Some("cust-1".to_owned());
    info.user_id = Some("user-1".to_owned());
    info.team_id = Some("team-1".to_owned());
    info.team_alias = Some("Team One".to_owned());
    info.organization_id = Some("org-1".to_owned());
    info.user_email = Some("u@example.com".to_owned());
    info.key_alias = Some("key-1".to_owned());
    info.projected_exceeded_date = Some("2026-02-01".to_owned());
    info.projected_spend = Some(decimal("900.5"));
    info.event_group = PortEntityType::Team;
    info.alert_emails = Some(vec!["a@example.com".to_owned()]);
    info.max_budget_alert_emails = Some(BTreeMap::from([(
        "50".to_owned(),
        vec!["a@example.com".to_owned()],
    )]));
    info
}

#[test]
fn every_present_field_reaches_the_event() {
    let info = full_info();

    let event = budget_webhook_event(
        &info,
        WebhookEventType::BudgetCrossed,
        "Budget Crossed".to_owned(),
    );

    assert_eq!(event.event, WebhookEventType::BudgetCrossed);
    assert_eq!(event.event_message, "Budget Crossed");
    assert_eq!(event.event_group, EntityType::Team);
    assert_eq!(event.spend, decimal("300"));
    assert_eq!(event.max_budget, Some(decimal("1000")));
    assert_eq!(event.soft_budget, Some(decimal("500")));
    let token = event.token.as_ref().expect("the event carries the key");
    assert_eq!(token.expose_secret(), "hashed-key");
}

#[test]
fn every_optional_field_reaches_the_event() {
    let info = full_info();

    let event = budget_webhook_event(
        &info,
        WebhookEventType::BudgetCrossed,
        "Budget Crossed".to_owned(),
    );

    assert_eq!(event.customer_id.as_deref(), Some("cust-1"));
    assert_eq!(event.user_id.as_deref(), Some("user-1"));
    assert_eq!(event.team_id.as_deref(), Some("team-1"));
    assert_eq!(event.team_alias.as_deref(), Some("Team One"));
    assert_eq!(event.organization_id.as_deref(), Some("org-1"));
    assert_eq!(event.user_email.as_deref(), Some("u@example.com"));
    assert_eq!(event.key_alias.as_deref(), Some("key-1"));
    assert_eq!(event.projected_exceeded_date.as_deref(), Some("2026-02-01"));
    assert_eq!(event.projected_spend, Some(decimal("900.5")));
    assert_eq!(event.alert_emails, Some(vec!["a@example.com".to_owned()]));
    assert_eq!(
        event.max_budget_alert_emails,
        Some(BTreeMap::from([(
            "50".to_owned(),
            vec!["a@example.com".to_owned()]
        )]))
    );
}

#[test]
fn an_absent_field_stays_absent() {
    let info = call_info(decimal("1"));

    let event = budget_webhook_event(&info, WebhookEventType::ThresholdCrossed, "m".to_owned());

    assert_eq!(event.event_group, EntityType::Key);
    assert!(event.token.is_none());
    assert!(event.max_budget.is_none());
    assert!(event.soft_budget.is_none());
    assert!(event.customer_id.is_none());
    assert!(event.team_alias.is_none());
    assert!(event.projected_spend.is_none());
    assert!(event.alert_emails.is_none());
    assert!(event.max_budget_alert_emails.is_none());
}

#[test]
fn the_budget_alert_message_carries_the_user_block_but_the_event_does_not() {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = PortEntityType::User;

    let request = budget_alert(BudgetAlertKind::UserBudget, &info).expect("a crossed budget alert");

    assert_eq!(request.event, WebhookEventType::BudgetCrossed);
    assert_eq!(
        request.webhook_event.event_message,
        "User Budget: Budget Crossed\n Total Budget:`100`"
    );
    assert_eq!(
        request.message,
        "User Budget: Budget Crossed\n Total Budget:`100`\n\n*spend:* `100`\n*max_budget:* `100`\n*user_id:* `user-1`\n*event_group:* `user`\n"
    );
}

#[test]
fn no_event_builds_no_webhook_event() {
    let mut info = call_info(decimal("10"));
    info.max_budget = Some(decimal("100"));

    assert!(budget_alert(BudgetAlertKind::UserBudget, &info).is_none());
}

#[rstest]
#[case(PortEntityType::Key, EntityType::Key)]
#[case(PortEntityType::User, EntityType::User)]
#[case(PortEntityType::EndUser, EntityType::EndUser)]
#[case(PortEntityType::Team, EntityType::Team)]
#[case(PortEntityType::TeamMember, EntityType::TeamMember)]
#[case(PortEntityType::Organization, EntityType::Organization)]
#[case(PortEntityType::OrganizationMember, EntityType::OrganizationMember)]
#[case(PortEntityType::Project, EntityType::Project)]
#[case(PortEntityType::Tag, EntityType::Tag)]
#[case(PortEntityType::Agent, EntityType::Agent)]
#[case(PortEntityType::ModelAccessGroup, EntityType::ModelAccessGroup)]
#[case(PortEntityType::Proxy, EntityType::Proxy)]
fn every_entity_group_maps_to_its_logging_twin(
    #[case] port: PortEntityType,
    #[case] expected: EntityType,
) {
    assert_eq!(entity_type(port), expected);
}
