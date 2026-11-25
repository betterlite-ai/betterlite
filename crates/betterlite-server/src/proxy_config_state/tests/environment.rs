use std::collections::BTreeMap;

use betterlite_config::sections::settings::Section;
use serde_json::json;

use super::support;
use crate::proxy_config_state::save;

fn updates(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.map(str::to_owned)))
        .collect()
}

#[tokio::test]
async fn save_environment_variables_merges_seals_and_deletes() {
    let database = support::MemoryDatabase::with_section(
        Section::EnvironmentVariables,
        support::values(&[("KEEP", json!("old")), ("DROP", json!("gone"))]),
    );
    let state = support::database_state();
    let updates = updates(&[
        ("KEEP", Some("new")),
        ("ADD", Some("value")),
        ("DROP", None),
    ]);
    save::save_environment_variables(&state, &updates, Some(&database))
        .await
        .unwrap();
    assert_eq!(
        database.stored(Section::EnvironmentVariables),
        Some(support::values(&[
            ("ADD", json!("sealed:value")),
            ("KEEP", json!("sealed:new")),
        ]))
    );
    assert_eq!(database.invalidated(), vec!["environment_variables"]);
}

#[tokio::test]
async fn save_environment_variables_leaves_the_loaded_store_alone() {
    let database = support::MemoryDatabase::with_section(
        Section::EnvironmentVariables,
        support::values(&[("KEEP", json!("old"))]),
    );
    let state = support::database_state();
    let updates = updates(&[("KEEP", Some("new"))]);
    save::save_environment_variables(&state, &updates, Some(&database))
        .await
        .unwrap();
    assert_eq!(state.environment_variables.get("KEEP"), None);
}

#[tokio::test]
async fn save_environment_variables_is_a_no_op_without_a_database() {
    let database = support::MemoryDatabase::default();
    let state = support::database_state();
    let updates = updates(&[("ADD", Some("value"))]);
    save::save_environment_variables(&state, &updates, None)
        .await
        .unwrap();
    assert!(database.inserted().is_empty());
    assert!(database.invalidated().is_empty());
}

#[tokio::test]
async fn save_environment_variables_is_a_no_op_while_store_model_in_db_is_off() {
    let database = support::MemoryDatabase::default();
    let state = support::state();
    let updates = updates(&[("ADD", Some("value"))]);
    save::save_environment_variables(&state, &updates, Some(&database))
        .await
        .unwrap();
    assert_eq!(database.stored(Section::EnvironmentVariables), None);
    assert!(database.invalidated().is_empty());
}
