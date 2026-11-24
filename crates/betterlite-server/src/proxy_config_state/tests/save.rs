use betterlite_config::sections::settings::{JsonValue, Section};
use tokio::fs;

use super::support::{self, json};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::save;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;
use crate::proxy_config_state::state::ProxyConfigState;

fn state_with_snapshot(config: &[(&str, JsonValue)]) -> ProxyConfigState {
    let mut state = support::database_state();
    state.replace_snapshot(&support::values(config));
    state
}

fn config_of(pairs: &[(&str, JsonValue)]) -> ConfigWithBaseline {
    ConfigWithBaseline::from_values(support::values(pairs))
}

#[tokio::test]
async fn save_config_writes_only_the_sections_that_changed() {
    let database = support::MemoryDatabase::default();
    let mut state = state_with_snapshot(&[
        ("general_settings", json!({"a": 1})),
        ("router_settings", json!({"b": 2})),
        ("litellm_settings", json!({"c": 3})),
    ]);
    let mut config = config_of(&[
        ("general_settings", json!({"a": 5})),
        ("router_settings", json!({"b": 2})),
        ("litellm_settings", json!({"c": 3})),
    ]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    let merged = database.merged();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].section, Section::GeneralSettings);
    assert_eq!(merged[0].changed, support::values(&[("a", json!(5))]));
    assert_eq!(database.invalidated(), vec!["general_settings"]);
    assert_eq!(
        database.stored(Section::GeneralSettings),
        Some(support::values(&[("a", json!(5))]))
    );
}

#[tokio::test]
async fn a_written_section_is_applied_to_its_store() {
    let database = support::MemoryDatabase::with_section(
        Section::GeneralSettings,
        support::values(&[("stored_only", json!(1))]),
    );
    let mut state = state_with_snapshot(&[("general_settings", json!({"a": 1}))]);
    let mut config = config_of(&[("general_settings", json!({"a": 2}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert_eq!(state.general_settings.get("a"), Some(&json!(2)));
    assert_eq!(state.general_settings.get("stored_only"), Some(&json!(1)));
}

#[tokio::test]
async fn a_section_whose_row_already_matches_leaves_the_store_and_the_cache_alone() {
    let database = support::MemoryDatabase::with_section(
        Section::GeneralSettings,
        support::values(&[("a", json!(2))]),
    );
    let mut state = state_with_snapshot(&[("general_settings", json!({"a": 1}))]);
    let mut config = config_of(&[("general_settings", json!({"a": 2}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert_eq!(database.merged().len(), 1);
    assert!(database.invalidated().is_empty());
    assert_eq!(state.general_settings.get("a"), None);
}

#[tokio::test]
async fn removing_a_key_drops_it_from_the_stored_section() {
    let database = support::MemoryDatabase::with_section(
        Section::RouterSettings,
        support::values(&[("gone", json!(1)), ("stays", json!(2))]),
    );
    let mut state = state_with_snapshot(&[("router_settings", json!({"gone": 1, "stays": 2}))]);
    let mut config = config_of(&[("router_settings", json!({"stays": 2}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert_eq!(
        database.stored(Section::RouterSettings),
        Some(support::values(&[("stays", json!(2))]))
    );
}

#[tokio::test]
async fn a_removed_config_owned_key_is_not_checked_by_the_write_guard() {
    let mut state = state_with_snapshot(&[("general_settings", json!({"a": 1, "b": 2}))]);
    state
        .general_settings
        .load_yaml(support::values(&[("a", json!(1)), ("b", json!(2))]));
    let database = support::MemoryDatabase::default();
    let mut config = config_of(&[("general_settings", json!({"a": 1}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    let merged = database.merged();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].removed.len(), 1);
    assert!(merged[0].removed.contains("b"));
}

#[tokio::test]
async fn a_config_owned_change_fails_before_the_database_is_touched() {
    let mut state = state_with_snapshot(&[("general_settings", json!({"master_key": "sk-file"}))]);
    state
        .general_settings
        .load_yaml(support::values(&[("master_key", json!("sk-file"))]));
    let database = support::MemoryDatabase::default();
    let mut config = config_of(&[("general_settings", json!({"master_key": "sk-new"}))]);
    let error = save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigStateError::ConfigOwned(_)));
    assert!(database.merged().is_empty());
    assert!(database.invalidated().is_empty());
}

#[tokio::test]
async fn save_config_inserts_top_level_keys_outside_the_persisted_sections() {
    let database = support::MemoryDatabase::default();
    let mut state = state_with_snapshot(&[
        ("general_settings", json!({"a": 1})),
        ("model_list", json!([{"model_name": "m"}])),
        ("environment_variables", json!({"E": "1"})),
        ("unchanged_key", json!("same")),
    ]);
    let mut config = config_of(&[
        ("general_settings", json!({"a": 1})),
        ("model_list", json!([{"model_name": "m"}])),
        ("environment_variables", json!({"E": "2"})),
        ("unchanged_key", json!("same")),
        ("new_key", json!("added")),
    ]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert_eq!(
        database.inserted(),
        vec![("new_key".to_owned(), json!("added"))]
    );
    assert_eq!(database.invalidated(), vec!["new_key"]);
}

#[tokio::test]
async fn save_config_with_env_vars_seals_the_environment_row() {
    let database = support::MemoryDatabase::default();
    let mut state = state_with_snapshot(&[("environment_variables", json!({}))]);
    let mut config = config_of(&[(
        "environment_variables",
        json!({"OPENAI_API_KEY": "sk-plain"}),
    )]);
    save::save_config(&mut state, &mut config, true, Some(&database))
        .await
        .unwrap();
    assert_eq!(
        database.inserted(),
        vec![(
            "environment_variables".to_owned(),
            json!({"OPENAI_API_KEY": "sealed:sk-plain"})
        )]
    );
    assert_eq!(database.invalidated(), vec!["environment_variables"]);
}

#[tokio::test]
async fn save_config_without_env_vars_leaves_the_environment_row_alone() {
    let database = support::MemoryDatabase::default();
    let mut state = state_with_snapshot(&[("environment_variables", json!({}))]);
    let mut config = config_of(&[(
        "environment_variables",
        json!({"OPENAI_API_KEY": "sk-plain"}),
    )]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert!(database.inserted().is_empty());
}

#[tokio::test]
async fn the_snapshot_and_a_carried_baseline_become_the_new_config() {
    let database = support::MemoryDatabase::default();
    let mut state = support::database_state();
    let mut config = ConfigWithBaseline::with_baseline(
        support::values(&[("general_settings", json!({"a": 2}))]),
        support::values(&[
            ("general_settings", json!({"a": 1})),
            ("kept_from_the_baseline", json!(true)),
        ]),
    );
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    let next = support::values(&[
        ("general_settings", json!({"a": 2})),
        ("kept_from_the_baseline", json!(true)),
    ]);
    assert_eq!(state.snapshot(), next);
    assert_eq!(config.baseline.as_ref(), Some(&next));
}

#[tokio::test]
async fn a_config_without_a_carried_baseline_diffs_against_the_snapshot() {
    let database = support::MemoryDatabase::default();
    let mut state = state_with_snapshot(&[("general_settings", json!({"a": 1}))]);
    let mut config = config_of(&[("general_settings", json!({"a": 1}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert!(database.merged().is_empty());
    assert!(config.baseline.is_none());
}

#[tokio::test]
async fn save_config_without_a_database_rewrites_the_config_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.yaml");
    fs::write(&path, "old: true\n").await.unwrap();
    let mut state = ProxyConfigState::new(path.clone(), true);
    state.replace_snapshot(&support::values(&[("old", json!(true))]));
    let mut config = config_of(&[("general_settings", json!({"a": 1}))]);
    save::save_config(&mut state, &mut config, false, None)
        .await
        .unwrap();
    let written = fs::read_to_string(&path).await.unwrap();
    let parsed: JsonValue = serde_saphyr::from_str(&written).unwrap();
    assert_eq!(parsed, json!({"general_settings": {"a": 1}}));
}

#[tokio::test]
async fn save_config_with_store_model_in_db_off_rewrites_the_config_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.yaml");
    let database = support::MemoryDatabase::default();
    let mut state = ProxyConfigState::new(path.clone(), false);
    state.replace_snapshot(&support::values(&[("general_settings", json!({"a": 1}))]));
    let mut config = config_of(&[("general_settings", json!({"a": 2}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert!(database.merged().is_empty());
    let written = fs::read_to_string(&path).await.unwrap();
    let parsed: JsonValue = serde_saphyr::from_str(&written).unwrap();
    assert_eq!(parsed, json!({"general_settings": {"a": 2}}));
}

#[tokio::test]
async fn the_general_settings_key_turns_database_writes_on() {
    let database = support::MemoryDatabase::default();
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("store_model_in_db", json!(true))]));
    let mut config = config_of(&[("general_settings", json!({"store_model_in_db": true}))]);
    save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap();
    assert_eq!(database.merged().len(), 1);
}

#[tokio::test]
async fn a_section_that_is_not_an_object_is_rejected() {
    let database = support::MemoryDatabase::default();
    let mut state = support::database_state();
    let mut config = config_of(&[("general_settings", json!("not an object"))]);
    let error = save::save_config(&mut state, &mut config, false, Some(&database))
        .await
        .unwrap_err();
    let ConfigStateError::InvalidSetting { field, expected } = error else {
        panic!("expected an invalid-setting error");
    };
    assert_eq!(field, "general_settings");
    assert_eq!(expected, "an object");
    assert!(database.merged().is_empty());
}

#[tokio::test]
async fn an_environment_section_that_is_not_an_object_is_rejected() {
    let database = support::MemoryDatabase::default();
    let mut state = support::database_state();
    let mut config = config_of(&[("environment_variables", json!(["nope"]))]);
    let error = save::save_config(&mut state, &mut config, true, Some(&database))
        .await
        .unwrap_err();
    assert!(matches!(error, ConfigStateError::InvalidSetting { .. }));
}

#[tokio::test]
async fn a_null_environment_section_inserts_nothing() {
    let database = support::MemoryDatabase::default();
    let mut state = support::database_state();
    let mut config = config_of(&[("environment_variables", json!(null))]);
    save::save_config(&mut state, &mut config, true, Some(&database))
        .await
        .unwrap();
    assert!(database.inserted().is_empty());
}
