#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_key_budget() {
    assert_eq!(
        BudgetAlertType::TokenBudget.get_event_message(),
        "Key Budget: "
    );
}

#[test]
fn the_id_is_the_key() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Key,
    };
    assert_eq!(BudgetAlertType::TokenBudget.get_id(&subject), "hashed-key");
}

#[test]
fn a_team_member_key_is_keyed_by_user_and_team() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::TeamMember,
    };
    assert_eq!(
        BudgetAlertType::TokenBudget.get_id(&subject),
        "team_member:user-1:team-1"
    );
}

#[test]
fn a_team_member_missing_an_id_still_names_both_slots() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: None,
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::TeamMember,
    };
    assert_eq!(
        BudgetAlertType::TokenBudget.get_id(&subject),
        "team_member::team-1"
    );
}

#[test]
fn a_missing_key_falls_back() {
    let subject = BudgetAlertSubject {
        token: None,
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::Key,
    };
    assert_eq!(BudgetAlertType::TokenBudget.get_id(&subject), "default_id");
}
