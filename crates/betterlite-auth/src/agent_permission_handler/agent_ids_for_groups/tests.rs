#![cfg(test)]

use crate::agent_permission_handler::AgentGrantsApi as _;
use crate::agent_permission_handler::tests::support::{handler, registry, set, strings, world};

#[tokio::test]
async fn the_public_lookup_unions_the_config_and_db_sources() {
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
