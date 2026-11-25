use std::collections::BTreeMap;

use betterlite_config::sections::settings::{DbRow, JsonValue, Section};

use super::support::{self, json};
use crate::proxy_config_state::error::{ConfigOwnedDetail, ConfigStateError};
use crate::proxy_config_state::state::ProxyConfigState;

fn owned_detail(
    state: &ProxyConfigState,
    section: Section,
    changed: &BTreeMap<String, JsonValue>,
) -> ConfigOwnedDetail {
    let error = state
        .reject_config_owned_writes(section, changed)
        .unwrap_err();
    let ConfigStateError::ConfigOwned(detail) = error else {
        panic!("expected a config-owned error");
    };
    *detail
}

#[test]
fn one_config_owned_key_is_refused_with_litellms_detail() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("master_key", json!("sk-file"))]));
    let changed = support::values(&[("master_key", json!("sk-new"))]);
    let detail = owned_detail(&state, Section::GeneralSettings, &changed);
    assert_eq!(
        detail.error,
        "general_settings key 'master_key' is set in the config file and cannot be changed here."
    );
    assert_eq!(detail.keys, vec!["master_key".to_owned()]);
    assert_eq!(detail.section, "general_settings");
    assert!(detail.stored_database_values_ignored.is_empty());
    assert_eq!(
        detail.resolution,
        format!(
            "edit {} to change it, or remove it from the file to let the database own it",
            support::CONFIG_PATH
        )
    );
}

#[test]
fn two_config_owned_keys_are_named_sorted_and_quoted() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("beta", json!(1)), ("alpha", json!(2))]));
    let changed = support::values(&[("beta", json!(3)), ("alpha", json!(4))]);
    let detail = owned_detail(&state, Section::GeneralSettings, &changed);
    assert_eq!(
        detail.error,
        "general_settings keys 'alpha', 'beta' are set in the config file and cannot be changed here."
    );
    assert_eq!(detail.keys, vec!["alpha".to_owned(), "beta".to_owned()]);
    assert_eq!(
        detail.resolution,
        format!(
            "edit {} to change them, or remove them from the file to let the database own them",
            support::CONFIG_PATH
        )
    );
}

#[test]
fn a_shadowed_stored_value_is_named_in_the_detail() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("master_key", json!("sk-file"))]));
    state.general_settings.apply_db_row(
        DbRow::GeneralSettings,
        support::values(&[("master_key", json!("sk-stored"))]),
    );
    let changed = support::values(&[("master_key", json!("sk-new"))]);
    let detail = owned_detail(&state, Section::GeneralSettings, &changed);
    assert_eq!(
        detail.error,
        "general_settings key 'master_key' is set in the config file and cannot be changed here. \
         The value already stored in the database for master_key is ignored and will never be applied."
    );
    assert_eq!(
        detail.stored_database_values_ignored,
        vec!["master_key".to_owned()]
    );
}

#[test]
fn two_shadowed_stored_values_are_named_together() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("alpha", json!(1)), ("beta", json!(2))]));
    state.general_settings.apply_db_row(
        DbRow::GeneralSettings,
        support::values(&[("alpha", json!(8)), ("beta", json!(9))]),
    );
    let changed = support::values(&[("alpha", json!(3)), ("beta", json!(4))]);
    let detail = owned_detail(&state, Section::GeneralSettings, &changed);
    assert_eq!(
        detail.error,
        "general_settings keys 'alpha', 'beta' are set in the config file and cannot be changed here. \
         The values already stored in the database for alpha, beta are ignored and will never be applied."
    );
}

#[test]
fn resending_the_config_files_value_is_not_refused() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("master_key", json!("sk-file"))]));
    let changed = support::values(&[("master_key", json!("sk-file"))]);
    state
        .reject_config_owned_writes(Section::GeneralSettings, &changed)
        .unwrap();
}

#[test]
fn a_key_the_config_file_does_not_set_is_written() {
    let state = support::state();
    let changed = support::values(&[("database_key", json!("value"))]);
    state
        .reject_config_owned_writes(Section::GeneralSettings, &changed)
        .unwrap();
}

#[test]
fn a_config_owned_delete_is_refused_even_for_the_current_value() {
    let mut state = support::state();
    state.litellm_settings.load_yaml(support::values(&[
        ("drop_params", json!(true)),
        ("num_retries", json!(2)),
    ]));
    let keys = vec!["drop_params".to_owned(), "database_key".to_owned()];
    let error = state
        .reject_config_owned_deletes(Section::LitellmSettings, &keys)
        .unwrap_err();
    let ConfigStateError::ConfigOwned(detail) = error else {
        panic!("expected a config-owned error");
    };
    assert_eq!(detail.keys, vec!["drop_params".to_owned()]);
    assert_eq!(detail.section, "litellm_settings");
}

#[test]
fn deleting_only_database_keys_passes_the_guard() {
    let state = support::state();
    let keys = vec!["database_key".to_owned(), "another_key".to_owned()];
    state
        .reject_config_owned_deletes(Section::LitellmSettings, &keys)
        .unwrap();
}

#[test]
fn ui_settings_has_no_store_so_both_guards_pass() {
    let state = support::state();
    let changed = support::values(&[("anything", json!(1))]);
    state
        .reject_config_owned_writes(Section::UiSettings, &changed)
        .unwrap();
    let keys = vec!["anything".to_owned()];
    state
        .reject_config_owned_deletes(Section::UiSettings, &keys)
        .unwrap();
}
