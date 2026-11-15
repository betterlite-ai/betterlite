#![cfg(test)]

use crate::budget_email::event::EntityType;
use crate::helpers::budget_alert_types::{
    BaseBudgetAlertType as _, BudgetAlertSubject, BudgetAlertType,
};

#[test]
fn message_names_a_projected_limit() {
    assert_eq!(
        BudgetAlertType::ProjectedLimitExceeded.get_event_message(),
        "Key Budget: Projected Limit Exceeded"
    );
}

#[test]
fn the_id_is_the_key() {
    let subject = BudgetAlertSubject {
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        organization_id: None,
        event_group: EntityType::TeamMember,
    };
    assert_eq!(
        BudgetAlertType::ProjectedLimitExceeded.get_id(&subject),
        "hashed-key",
        "only a key budget alert switches to the team member id"
    );
}

#[test]
fn a_missing_key_falls_back() {
    let subject = BudgetAlertSubject {
        token: None,
        user_id: Some("user-1"),
        team_id: None,
        organization_id: None,
        event_group: EntityType::Key,
    };
    assert_eq!(
        BudgetAlertType::ProjectedLimitExceeded.get_id(&subject),
        "default_id"
    );
}
