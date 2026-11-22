#![cfg(test)]

use super::db_agent_ids;
use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::error::GrantError;
use crate::agent_permission_handler::tests::support::{handler, registry, set, strings, world};

#[tokio::test]
async fn rows_naming_a_declared_group_match() {
    let world = world(true);
    {
        let mut state = world.lock();
        state.rows.push((String::from("alpha"), strings(&["g1"])));
        state.rows.push((String::from("beta"), strings(&["g2"])));
    };
    let found = db_agent_ids(&world, &strings(&["g1"]), false)
        .await
        .unwrap();
    assert_eq!(found, set(&["alpha"]));
    assert_eq!(world.lock().table_reads, vec![(strings(&["g1"]), false)]);
}

#[tokio::test]
async fn no_database_reads_no_rows() {
    let world = world(false);
    world
        .lock()
        .rows
        .push((String::from("alpha"), strings(&["g1"])));
    let found = db_agent_ids(&world, &strings(&["g1"]), true).await.unwrap();
    assert!(found.is_empty());
    assert!(world.lock().table_reads.is_empty());
}

#[tokio::test]
async fn no_declared_group_reads_no_rows() {
    let world = world(true);
    let found = db_agent_ids(&world, &[], true).await.unwrap();
    assert!(found.is_empty());
    assert!(world.lock().table_reads.is_empty());
}

#[tokio::test]
async fn the_public_lookup_returns_the_matching_table_rows() {
    let world = world(true);
    world
        .lock()
        .rows
        .push((String::from("alpha"), strings(&["g1"])));
    let grants = handler(&world, &registry(&[]));
    let found = grants
        .agent_ids_for_groups(&strings(&["g1"]), true)
        .await
        .unwrap();
    assert_eq!(found, set(&["alpha"]));
    assert_eq!(world.lock().table_reads, vec![(strings(&["g1"]), true)]);
}

#[tokio::test]
async fn a_table_failure_becomes_a_lookup_error() {
    let world = world(true);
    world.lock().failures.table = true;
    let error = db_agent_ids(&world, &strings(&["g1"]), true)
        .await
        .unwrap_err();
    let GrantError::Lookup(_) = error else {
        panic!("lookup error expected, got {error:?}");
    };
}
