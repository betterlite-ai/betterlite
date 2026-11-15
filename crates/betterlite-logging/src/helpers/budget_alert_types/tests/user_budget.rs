#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_user_budget() {
    assert_eq!(
        BudgetAlertType::UserBudget.get_event_message(),
        "User Budget: "
    );
}

#[test]
fn the_id_is_the_user() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::User,
    };
    assert_eq!(BudgetAlertType::UserBudget.get_id(&subject), "user-1");
}

#[test]
fn a_missing_user_falls_back() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: None,
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::User,
    };
    assert_eq!(BudgetAlertType::UserBudget.get_id(&subject), "default_id");
}
