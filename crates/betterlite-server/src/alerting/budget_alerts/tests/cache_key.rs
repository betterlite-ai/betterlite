#![cfg(test)]

use crate::failure_hook::ports::{BudgetAlertKind, EntityType};

use super::support::{call_info, decimal, written_key};

#[tokio::test]
async fn a_crossed_user_budget_is_keyed_by_event_and_user() {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;

    assert_eq!(
        written_key(BudgetAlertKind::UserBudget, &info).await,
        "budget_alerts:budget_crossed:user-1"
    );
}

#[tokio::test]
async fn a_crossed_soft_budget_is_keyed_by_the_team() {
    let mut info = call_info(decimal("50"));
    info.soft_budget = Some(decimal("50"));
    info.team_id = Some("team-1".to_owned());
    info.event_group = EntityType::Team;

    assert_eq!(
        written_key(BudgetAlertKind::SoftBudget, &info).await,
        "budget_alerts:soft_budget_crossed:team-1"
    );
}

#[tokio::test]
async fn a_crossed_threshold_is_keyed_by_event_and_user() {
    let mut info = call_info(decimal("96"));
    info.max_budget = Some(decimal("100"));
    info.user_id = Some("user-1".to_owned());
    info.event_group = EntityType::User;

    assert_eq!(
        written_key(BudgetAlertKind::UserBudget, &info).await,
        "budget_alerts:threshold_crossed:user-1"
    );
}

#[tokio::test]
async fn a_projected_limit_is_keyed_by_event_and_key() {
    let mut info = call_info(decimal("10"));
    info.max_budget = Some(decimal("100"));
    info.token = Some("hashed-key".to_owned());

    assert_eq!(
        written_key(BudgetAlertKind::ProjectedLimitExceeded, &info).await,
        "budget_alerts:projected_limit_exceeded:hashed-key"
    );
}

#[tokio::test]
async fn a_team_member_key_budget_names_both_ids_in_the_key() {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));
    info.token = Some("hashed-key".to_owned());
    info.user_id = Some("user-1".to_owned());
    info.team_id = Some("team-1".to_owned());
    info.event_group = EntityType::TeamMember;

    assert_eq!(
        written_key(BudgetAlertKind::TokenBudget, &info).await,
        "budget_alerts:budget_crossed:team_member:user-1:team-1"
    );
}

#[tokio::test]
async fn a_proxy_budget_uses_the_default_id() {
    let mut info = call_info(decimal("100"));
    info.max_budget = Some(decimal("100"));

    assert_eq!(
        written_key(BudgetAlertKind::ProxyBudget, &info).await,
        "budget_alerts:budget_crossed:default_id"
    );
}
