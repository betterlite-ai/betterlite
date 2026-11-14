#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_an_organization_budget() {
    assert_eq!(
        BudgetAlertType::OrganizationBudget.get_event_message(),
        "Organization Budget: "
    );
}

#[test]
fn the_id_is_the_organization() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: Some("org-1"),
        event_group: EntityType::Organization,
    };
    assert_eq!(
        BudgetAlertType::OrganizationBudget.get_id(&subject),
        "org-1"
    );
}

#[test]
fn a_missing_organization_falls_back() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Organization,
    };
    assert_eq!(
        BudgetAlertType::OrganizationBudget.get_id(&subject),
        "default_id"
    );
}
