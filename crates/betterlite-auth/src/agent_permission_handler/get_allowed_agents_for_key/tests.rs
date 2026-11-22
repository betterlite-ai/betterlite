#![cfg(test)]

use super::allowed_agents_for_key;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{
    Warnings, assert_policy_unavailable, group_row, handler, key_auth, registry, set, strings,
    world,
};

#[tokio::test]
async fn no_auth_is_unrestricted() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let access = allowed_agents_for_key(&grants, None, true).await.unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
}

#[tokio::test]
async fn an_undeclared_key_is_unrestricted_without_a_read() {
    let world = world(false);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(Some("t1"), None, &[]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert!(world.lock().table_reads.is_empty());
}

#[tokio::test]
async fn direct_agents_are_restricted() {
    let world = world(false);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&["alpha"], &[])), &[]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
}

#[tokio::test]
async fn declared_groups_add_matching_db_agents() {
    let world = world(true);
    let mut state = world.lock();
    state.rows.push((String::from("alpha"), strings(&["g1"])));
    state.rows.push((String::from("beta"), strings(&["g2"])));
    drop(state);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&[], &["g1"])), &[]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
    assert_eq!(world.lock().table_reads, vec![(strings(&["g1"]), false)]);
}

#[tokio::test]
async fn unified_groups_add_their_agents() {
    let world = world(true);
    world
        .lock()
        .groups
        .insert(String::from("u1"), group_row("u1", &["beta"]));
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, None, &["u1"]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["beta"])));
}

#[tokio::test]
async fn all_three_sources_union() {
    let world = world(true);
    let mut state = world.lock();
    state
        .rows
        .push((String::from("db-agent"), strings(&["g1"])));
    state
        .groups
        .insert(String::from("u1"), group_row("u1", &["unified-agent"]));
    drop(state);
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&["direct-agent"], &["g1"])), &["u1"]);
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(
        access,
        AgentAccess::Restricted(set(&["direct-agent", "db-agent", "unified-agent"]))
    );
}

#[tokio::test]
async fn a_strict_lookup_failure_is_policy_unavailable() {
    let world = world(true);
    world.lock().failures.table = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&[], &["g1"])), &[]);
    let error = grants
        .get_allowed_agents_for_key(Some(&auth), true)
        .await
        .unwrap_err();
    assert_policy_unavailable(&error);
}

#[tokio::test]
async fn a_non_strict_lookup_failure_warns_and_allows() {
    let world = world(true);
    world.lock().failures.table = true;
    let grants = handler(&world, &registry(&[]));
    let auth = key_auth(None, Some((&[], &["g1"])), &[]);
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = grants
        .get_allowed_agents_for_key(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(warnings.count(), 1);
}
