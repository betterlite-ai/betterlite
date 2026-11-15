#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_soft_budget() {
    assert_eq!(
        BudgetAlertType::SoftBudget.get_event_message(),
        "Soft Budget Crossed: "
    );
}

#[test]
fn a_team_alert_is_keyed_by_its_team() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Team,
    };
    assert_eq!(BudgetAlertType::SoftBudget.get_id(&subject), "team-1");
}

#[test]
fn a_team_alert_without_a_team_falls_back() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: None,
        team_id: None,
        organization_id: None,
        event_group: EntityType::Team,
    };
    assert_eq!(
        BudgetAlertType::SoftBudget.get_id(&subject),
        "default_id",
        "a missing team id must not borrow the key id"
    );
}

#[test]
fn any_other_group_is_keyed_by_its_key() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Key,
    };
    assert_eq!(BudgetAlertType::SoftBudget.get_id(&subject), "hashed-key");
}

#[test]
fn a_keyless_alert_falls_back() {
    let subject = BudgetAlertSubject {
        token: None,
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::User,
    };
    assert_eq!(BudgetAlertType::SoftBudget.get_id(&subject), "default_id");
}
