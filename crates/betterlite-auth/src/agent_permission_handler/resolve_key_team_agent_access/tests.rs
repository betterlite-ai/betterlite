#![cfg(test)]

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{
    assert_policy_unavailable, handler, key_auth, registry, set, team_row, world,
};

#[tokio::test]
async fn key_and_team_grants_intersect() {
    let world = world(true);
    world.lock().team = Some(team_row(&["beta", "gamma"], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), Some((&["alpha", "beta"], &[])), &[]);
    let access = grants
        .resolve_key_team_agent_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["beta"])));
}

#[tokio::test]
async fn an_unrestricted_side_keeps_the_other() {
    let world = world(true);
    world.lock().team = Some(team_row(&[], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), Some((&["alpha"], &[])), &[]);
    let access = grants
        .resolve_key_team_agent_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
}

#[tokio::test]
async fn both_sides_unrestricted_stays_unrestricted() {
    let world = world(false);
    let grants = handler(&world, &registry(&[]));
    let access = grants
        .resolve_key_team_agent_access(None, true)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
}

#[tokio::test]
async fn disjoint_grants_deny_everything() {
    let world = world(true);
    world.lock().team = Some(team_row(&["beta"], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), Some((&["alpha"], &[])), &[]);
    let access = grants
        .resolve_key_team_agent_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn a_strict_team_failure_is_policy_unavailable() {
    let world = world(true);
    world.lock().failures.team = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let error = grants
        .resolve_key_team_agent_access(Some(&auth), true)
        .await
        .unwrap_err();
    assert_policy_unavailable(&error);
}

#[tokio::test]
async fn a_non_strict_team_failure_allows_the_key_access() {
    let world = world(true);
    world.lock().failures.team = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), Some((&["alpha"], &[])), &[]);
    let access = grants
        .resolve_key_team_agent_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
}
