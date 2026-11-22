#![cfg(test)]

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{
    group_row, handler, key_auth, registry, set, team_row, world,
};

#[tokio::test]
async fn an_empty_group_denies_a_key() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&[], &["g1"])), &[]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn a_missing_group_denies_a_key() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&[], &["never-created"])), &[]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn an_empty_unified_group_denies_a_key() {
    let world = world(true);
    world
        .lock()
        .groups
        .insert(String::from("u1"), group_row("u1", &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, None, &["u1"]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn a_missing_unified_group_denies_a_key() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, None, &["never-created"]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn an_empty_group_denies_a_team() {
    let world = world(true);
    world.lock().team = Some(team_row(&[], &["g1"], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}

#[tokio::test]
async fn a_missing_group_denies_a_team() {
    let world = world(true);
    world.lock().team = Some(team_row(&[], &[], &["never-created"]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_team(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&[])));
}
