#![cfg(test)]

use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::error::GrantError;
use crate::agent_permission_handler::tests::support::{
    agent_auth, group_row, handler, key_auth, registry, set, strings, world,
};
use crate::agent_registry::AgentRegistryApi as _;

#[tokio::test]
async fn guard_without_auth_or_agent_id_returns_none() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let none = grants.agent_access_group_ceiling(None, true).await.unwrap();
    assert_eq!(none, None);
    let auth = key_auth(Some("t1"), None, &[]);
    let no_agent = grants
        .agent_access_group_ceiling(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(no_agent, None);
    assert!(world.lock().group_reads.is_empty());
}

#[tokio::test]
async fn null_without_declared_groups_returns_none() {
    let world = world(true);
    let grants = handler(&world, &registry(&[]));
    let auth = agent_auth("alpha");
    let ceiling = grants
        .agent_access_group_ceiling(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(ceiling, None);
    assert!(world.lock().group_reads.is_empty());
}

#[tokio::test]
async fn dispatch_reads_the_writer_only_when_strict() {
    let world = world(true);
    {
        let mut state = world.lock();
        state
            .agent_groups
            .insert(String::from("alpha"), strings(&["g1"]));
        state
            .groups
            .insert(String::from("g1"), group_row("g1", &["beta"]));
    }
    let grants = handler(&world, &registry(&[]));
    let auth = agent_auth("alpha");
    let strict = grants
        .agent_access_group_ceiling(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(strict, Some(set(&["beta"])));
    let loose = grants
        .agent_access_group_ceiling(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(loose, Some(set(&["beta"])));
    assert_eq!(
        world.lock().group_reads,
        vec![(String::from("g1"), true), (String::from("g1"), false)]
    );
}

#[tokio::test]
async fn stable_ids_map_legacy_group_members_to_the_registry_id() {
    let world = world(true);
    let agents = registry(&[r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#]);
    let stable = agents.get_agent_list(None)[0].agent_id.clone();
    let legacy = agents
        .ids_for_agent(&stable)
        .into_iter()
        .find(|id| id != &stable)
        .unwrap();
    {
        let mut state = world.lock();
        state
            .agent_groups
            .insert(String::from("kernel"), strings(&["g1"]));
        state
            .groups
            .insert(String::from("g1"), group_row("g1", &[legacy.as_str()]));
    }
    let grants = handler(&world, &agents);
    let auth = agent_auth("kernel");
    let ceiling = grants
        .agent_access_group_ceiling(Some(&auth), true)
        .await
        .unwrap();
    assert_eq!(ceiling, Some(set(&[stable.as_str()])));
}

#[tokio::test]
async fn a_strict_group_failure_is_a_lookup_error() {
    let world = world(true);
    world
        .lock()
        .agent_groups
        .insert(String::from("alpha"), strings(&["gone"]));
    let grants = handler(&world, &registry(&[]));
    let auth = agent_auth("alpha");
    let error = grants
        .agent_access_group_ceiling(Some(&auth), true)
        .await
        .unwrap_err();
    let GrantError::Lookup(detail) = error else {
        panic!("lookup error expected, got {error:?}");
    };
    assert!(detail.contains("gone"));
}

#[tokio::test]
async fn a_non_strict_group_failure_denies_through_an_empty_ceiling() {
    let world = world(true);
    world
        .lock()
        .agent_groups
        .insert(String::from("alpha"), strings(&["gone"]));
    let grants = handler(&world, &registry(&[]));
    let auth = agent_auth("alpha");
    let ceiling = grants
        .agent_access_group_ceiling(Some(&auth), false)
        .await
        .unwrap();
    assert_eq!(ceiling, Some(set(&[])));
}
