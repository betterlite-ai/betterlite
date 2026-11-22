#![cfg(test)]

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{
    Warnings, agent_auth, assert_policy_unavailable, bare_team, group_row, handler, key_auth,
    registry, set, strings, team_row, world,
};

#[tokio::test]
async fn no_auth_or_team_is_unrestricted() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let no_auth = grants
        .get_allowed_agents_for_team(None, true)
        .await
        .unwrap();
    assert_eq!(no_auth, AgentAccess::Unrestricted);
    let auth = key_auth(None, None, &[]);
    let no_team = grants
        .get_allowed_agents_for_team(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(no_team, AgentAccess::Unrestricted);
    assert!(world.lock().team_reads.is_empty());
}

#[tokio::test]
async fn team_direct_agents_are_restricted() {
    let world = world(true);
    world.lock().team = Some(team_row(&["alpha"], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
    assert_eq!(world.lock().team_reads, vec![(String::from("t1"), false)]);
}

#[tokio::test]
async fn a_team_without_grants_is_unrestricted() {
    let world = world(true);
    world.lock().team = Some(bare_team());
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
}

#[tokio::test]
async fn team_declared_groups_add_db_agents() {
    let world = world(true);
    {
        let mut state = world.lock();
        state.team = Some(team_row(&[], &["g1"], &[]));
        state.rows.push((String::from("alpha"), strings(&["g1"])));
    };
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
}

#[tokio::test]
async fn team_unified_groups_add_their_agents() {
    let world = world(true);
    {
        let mut state = world.lock();
        state.team = Some(team_row(&[], &[], &["u1"]));
        state
            .groups
            .insert(String::from("u1"), group_row("u1", &["beta"]));
    };
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["beta"])));
}

#[tokio::test]
async fn no_database_denies_in_strict_mode_and_allows_otherwise() {
    let world = world(false);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let strict = grants
        .get_allowed_agents_for_team(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(strict, AgentAccess::Restricted(set(&[])));
    let loose = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(loose, AgentAccess::Unrestricted);
    assert!(world.lock().team_reads.is_empty());
}

#[tokio::test]
async fn strict_mode_reads_the_team_from_the_writer() {
    let world = world(true);
    world.lock().team = Some(bare_team());
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(world.lock().team_reads, vec![(String::from("t1"), true)]);
}

#[tokio::test]
async fn a_missing_team_is_policy_unavailable_in_strict_mode() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let error = grants
        .get_allowed_agents_for_team(Some(&auth), true)
        .await
        .unwrap_err();
    assert_policy_unavailable(&error);
}

#[tokio::test]
async fn a_missing_team_warns_and_allows_outside_strict_mode() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(warnings.count(), 1);
}

#[tokio::test]
async fn the_ui_team_failure_skips_the_warning() {
    let world = world(true);
    world.lock().failures.team = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("litellm-dashboard"), None, &[]);
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(warnings.count(), 0);
}

#[tokio::test]
async fn a_strict_team_read_failure_is_policy_unavailable() {
    let world = world(true);
    world.lock().failures.team = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let error = grants
        .get_allowed_agents_for_team(Some(&auth), true)
        .await
        .unwrap_err();
    assert_policy_unavailable(&error);
}

#[tokio::test]
async fn an_agent_key_team_is_not_the_caller_team() {
    let world = world(true);
    world.lock().team = Some(team_row(&["alpha"], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = agent_auth("agents-key");
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert!(world.lock().team_reads.is_empty());
}
