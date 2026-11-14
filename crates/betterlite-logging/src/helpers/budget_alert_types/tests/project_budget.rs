#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_project_budget() {
    assert_eq!(
        BudgetAlertType::ProjectBudget.get_event_message(),
        "Project Budget: "
    );
}

#[test]
fn the_id_is_the_key() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Project,
    };
    assert_eq!(
        BudgetAlertType::ProjectBudget.get_id(&subject),
        "hashed-key"
    );
}

#[test]
fn a_missing_key_falls_back() {
    let subject = BudgetAlertSubject {
        token: None,
        user_id: None,
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Project,
    };
    assert_eq!(
        BudgetAlertType::ProjectBudget.get_id(&subject),
        "default_id"
    );
}
