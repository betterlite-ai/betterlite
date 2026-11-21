#![cfg(test)]

use super::access;
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{
    Warnings, assert_policy_unavailable, caller_auth, handler, key_auth, registry, set, team_row,
    world,
};

#[tokio::test]
async fn no_auth_or_caller_is_unrestricted() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let access = access(&grants, None, false).await.unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    let auth = key_auth(Some("t1"), Some((&["alpha"], &[])), &[]);
    let access = grants
        .agent_caller_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert!(world.lock().team_reads.is_empty());
}

#[tokio::test]
async fn the_caller_team_is_read_not_the_key_team() {
    let world = world(true);
    world.lock().team = Some(team_row(&["alpha"], &[], &[]));
    let grants = handler(&world, &registry(&[]));
    let auth = caller_auth("t2");
    let access = grants
        .agent_caller_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Restricted(set(&["alpha"])));
    assert_eq!(world.lock().team_reads, vec![(String::from("t2"), false)]);
}

#[tokio::test]
async fn a_strict_caller_team_failure_is_policy_unavailable() {
    let world = world(true);
    world.lock().team_fails = true;
    let grants = handler(&world, &registry(&[]));
    let auth = caller_auth("t2");
    let error = grants
        .agent_caller_access(Some(&auth), true)
        .await
        .unwrap_err();
    assert_policy_unavailable(&error);
}

#[tokio::test]
async fn a_non_strict_caller_team_failure_warns_and_allows() {
    let world = world(true);
    world.lock().team_fails = true;
    let grants = handler(&world, &registry(&[]));
    let auth = caller_auth("t2");
    let warnings = Warnings::default();
    let _guard = warnings.install();
    let access = grants
        .agent_caller_access(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(warnings.count(), 1);
}
