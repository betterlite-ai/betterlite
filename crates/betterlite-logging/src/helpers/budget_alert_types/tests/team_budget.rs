#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_team_budget() {
    assert_eq!(
        BudgetAlertType::TeamBudget.get_event_message(),
        "Team Budget: "
    );
}

#[test]
fn the_id_is_the_team() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Team,
    };
    assert_eq!(BudgetAlertType::TeamBudget.get_id(&subject), "team-1");
}

#[test]
fn a_missing_team_falls_back() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: None,
        organization_id: None,
        event_group: EntityType::Team,
    };
    assert_eq!(BudgetAlertType::TeamBudget.get_id(&subject), "default_id");
}
