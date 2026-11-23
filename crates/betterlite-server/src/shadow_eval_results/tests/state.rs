#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;

use super::super::postgres::store_from_state;
use crate::routes::tests::state_for;

fn configured_config() -> Config {
    let yaml = "general_settings:\n  database_url: postgresql://betterlite:betterlite@localhost:5432/betterlite\n";
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap()
}

// the route opens the writer pool the CLI resolved, not an empty placeholder
#[tokio::test]
async fn store_from_state_builds_a_store_from_the_configured_url() {
    let state = state_for(&configured_config());
    let store = store_from_state(&state).unwrap();
    assert!(store.store.pool.is_some());
}
