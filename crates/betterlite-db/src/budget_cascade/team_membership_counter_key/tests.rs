#![cfg(test)]

use super::super::linked_rows::{LinkedKeys as _, TeamMembershipRow};

fn row() -> TeamMembershipRow {
    TeamMembershipRow {
        user_id: "user-a".to_owned(),
        team_id: "team-a".to_owned(),
        budget_id: Some("budget-a".to_owned()),
        spend: None,
    }
}

#[test]
fn team_membership_counter_key_names_user_then_team() {
    assert_eq!(
        row().counter_key(),
        "spend:team_member:user-a:team-a".to_owned()
    );
}

#[test]
fn team_membership_cache_keys_name_the_team_first() {
    assert_eq!(row().cache_keys(), vec!["team-a_user-a".to_owned()]);
}
