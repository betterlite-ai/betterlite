#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{ConfigOwnedKeyError, Resolution, SettingsStore, config_ownership_message};
use crate::sections::settings::settings_resolution::FieldSource;
use crate::sections::settings::settings_rules::{DbRow, JsonValue, Section};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn values(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

fn general(yaml: &str, stored: &str) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(values(yaml));
    store.apply_db_row(DbRow::GeneralSettings, values(stored));
    store
}

#[rstest]
#[case::config_wins_over_db(r#"{"a": 1}"#, r#"{"a": 2}"#, Some("1"), FieldSource::Config)]
#[case::config_null_is_a_value(r#"{"a": null}"#, r#"{"a": 2}"#, Some("null"), FieldSource::Config)]
#[case::db_when_config_is_absent("{}", r#"{"a": 2}"#, Some("2"), FieldSource::Db)]
#[case::db_null_is_unset("{}", r#"{"a": null}"#, None, FieldSource::Unset)]
#[case::missing_everywhere("{}", "{}", None, FieldSource::Unset)]
fn resolves_config_then_db(
    #[case] yaml: &str,
    #[case] stored: &str,
    #[case] expected: Option<&str>,
    #[case] source: FieldSource,
) {
    let store = general(yaml, stored);
    assert_eq!(store.get("a"), expected.map(json).as_ref());
    assert_eq!(store.resolved().is_empty(), expected.is_none());
    assert_eq!(store.resolution("a").source(), source);
}

#[test]
fn runtime_values_win_over_resolution() {
    let mut store = general(r#"{"a": 1}"#, r#"{"b": 2}"#);
    store.apply_runtime_values(values(r#"{"a": "file", "b": "live", "c": 3}"#));
    assert_eq!(store.resolved(), values(r#"{"a": "file", "b": "live", "c": 3}"#));
    assert_eq!(store.resolution("a"), Resolution::Config(&json("1")));
    assert_eq!(store.resolution("b"), Resolution::Db(&json("2")));
    assert_eq!(store.resolution("c"), Resolution::Unset);
}

#[rstest]
#[case::without_stored_value("{}", false)]
#[case::with_stored_value(r#"{"owned": "db"}"#, true)]
#[case::with_stored_null(r#"{"owned": null}"#, false)]
fn insert_refuses_config_owned_key(#[case] stored: &str, #[case] shadows_db_value: bool) {
    let mut store = general(r#"{"owned": "yaml"}"#, stored);
    let expected = ConfigOwnedKeyError { section: Section::GeneralSettings, key: String::from("owned"), shadows_db_value };
    let message = config_ownership_message(Section::GeneralSettings, "owned", shadows_db_value);
    assert_eq!(expected.to_string(), message);
    assert_eq!(store.insert("owned", json(r#""other""#)), Err(expected));
    assert_eq!(store.insert("owned", json(r#""yaml""#)), Ok(()));
    assert_eq!(store.get("owned"), Some(&json(r#""yaml""#)));
}

#[rstest]
#[case::general(Section::GeneralSettings, false, "general_settings.k is set in the config file, so the config file owns it and it cannot be changed here. Edit the config file to change it, or remove it from the file to let the database own it.")]
#[case::router(Section::RouterSettings, true, "router_settings.k is set in the config file, so the config file owns it and it cannot be changed here. The value stored in the database for it is ignored and will never be applied. Edit the config file to change it, or remove it from the file to let the database own it.")]
#[case::litellm(Section::LitellmSettings, false, "litellm_settings.k is set in the config file, so the config file owns it and it cannot be changed here. Edit the config file to change it, or remove it from the file to let the database own it.")]
#[case::environment(Section::EnvironmentVariables, false, "environment_variables.k is set in the config file, so the config file owns it and it cannot be changed here. Edit the config file to change it, or remove it from the file to let the database own it.")]
fn message_names_the_section(#[case] section: Section, #[case] shadows: bool, #[case] expected: &str) {
    assert_eq!(config_ownership_message(section, "k", shadows), expected);
}

#[test]
fn remove_hides_the_key_everywhere() {
    let mut store = general("{}", r#"{"a": 1, "b": 2}"#);
    store.apply_runtime_values(values(r#"{"c": 3}"#));
    assert_eq!(store.remove("a"), Ok(Some(json("1"))));
    assert_eq!(store.remove("c"), Ok(Some(json("3"))));
    assert_eq!(store.get("a"), None);
    assert_eq!(store.resolved(), values(r#"{"b": 2}"#));
    assert_eq!(store.resolved().len(), 1);
    assert_eq!(store.remove("a"), Ok(None));
    assert_eq!(store.remove("b"), Ok(Some(json("2"))));
    assert!(store.resolved().is_empty());
    assert_eq!(store.insert("a", json("5")), Ok(()));
    assert_eq!(store.get("a"), Some(&json("5")));
}

#[test]
fn remove_refuses_config_owned_key() {
    let mut store = general(r#"{"owned": 1}"#, r#"{"owned": 2}"#);
    let expected = ConfigOwnedKeyError { section: Section::GeneralSettings, key: String::from("owned"), shadows_db_value: true };
    assert_eq!(store.remove("owned"), Err(expected));
    assert_eq!(store.remove("missing"), Ok(None));
    assert_eq!(store.get("owned"), Some(&json("1")));
}

#[test]
fn db_update_clears_only_changed_runtime_keys() {
    let mut store = general("{}", r#"{"a": 1, "b": 1, "gone": 1}"#);
    store.apply_runtime_values(values(r#"{"a": "rt", "b": "rt", "c": "rt", "gone": "rt"}"#));
    store.apply_db_row(DbRow::GeneralSettings, values(r#"{"a": 2, "b": 1}"#));
    assert_eq!(store.resolved(), values(r#"{"a": 2, "b": "rt", "c": "rt"}"#));
}

#[test]
fn db_update_keeps_config_owned_runtime_keys() {
    let mut store = general(r#"{"a": "yaml"}"#, r#"{"a": 1}"#);
    store.apply_runtime_values(values(r#"{"a": "resolved"}"#));
    store.apply_db_row(DbRow::GeneralSettings, values(r#"{"a": 2}"#));
    assert_eq!(store.get("a"), Some(&json(r#""resolved""#)));
}

#[test]
fn db_update_brings_back_a_removed_key_only_when_it_changes() {
    let mut store = general("{}", r#"{"a": 1, "b": 1}"#);
    assert_eq!(store.remove("a"), Ok(Some(json("1"))));
    assert_eq!(store.remove("b"), Ok(Some(json("1"))));
    store.apply_db_row(DbRow::GeneralSettings, values(r#"{"a": 2, "b": 1}"#));
    assert_eq!(store.resolved(), values(r#"{"a": 2}"#));
}

#[test]
fn reloads_forget_removed_keys() {
    let mut store = general("{}", r#"{"a": 1, "b": 2}"#);
    assert_eq!(store.remove("a"), Ok(Some(json("1"))));
    store.apply_runtime_values(BTreeMap::new());
    assert_eq!(store.get("a"), Some(&json("1")));
    assert_eq!(store.remove("b"), Ok(Some(json("2"))));
    store.load_yaml(BTreeMap::new());
    assert_eq!(store.get("b"), Some(&json("2")));
}

#[test]
fn load_yaml_drops_runtime_values_except_resource_lists() {
    let mut store = general("{}", "{}");
    store.apply_runtime_values(values(r#"{"a": 1, "pass_through_endpoints": [{"path": "/x"}]}"#));
    store.load_yaml(values(r#"{"b": 2}"#));
    assert_eq!(store.resolved(), values(r#"{"b": 2, "pass_through_endpoints": [{"path": "/x"}]}"#));
}

#[rstest]
#[case::readiness("allow_public_health_readiness_details")]
#[case::client_headers("forward_client_headers_to_llm_api")]
#[case::provider_auth_headers("forward_llm_provider_auth_headers")]
#[case::agents_for_internal_users("disable_agents_for_internal_users")]
#[case::agents_for_team_admins("allow_agents_for_team_admins")]
#[case::vector_stores_for_internal_users("disable_vector_stores_for_internal_users")]
#[case::vector_stores_for_team_admins("allow_vector_stores_for_team_admins")]
#[case::custom_api_keys("disable_custom_api_keys")]
#[case::org_admin_key_generate("disable_key_generate_for_org_admin")]
#[case::team_admin_fields("team_admin_editable_team_fields")]
fn ui_keys_read_the_ui_settings_row(#[case] key: &str) {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.apply_db_row(DbRow::GeneralSettings, BTreeMap::from([(key.to_owned(), json(r#""general""#))]));
    assert_eq!(store.get(key), None);
    let ui_row = BTreeMap::from([(key.to_owned(), json(r#""ui""#)), (String::from("master_key"), json("1"))]);
    store.apply_db_row(DbRow::UiSettings, ui_row);
    assert_eq!(store.resolved(), BTreeMap::from([(key.to_owned(), json(r#""ui""#))]));
}

#[rstest]
#[case::router(Section::RouterSettings, DbRow::RouterSettings)]
#[case::litellm(Section::LitellmSettings, DbRow::LitellmSettings)]
#[case::environment(Section::EnvironmentVariables, DbRow::EnvironmentVariables)]
fn other_sections_read_their_own_row(#[case] section: Section, #[case] row: DbRow) {
    let mut store = SettingsStore::from(section);
    store.load_yaml(values(r#"{"pass_through_endpoints": 1}"#));
    store.apply_db_row(DbRow::GeneralSettings, values(r#"{"disable_custom_api_keys": 1, "a": 1}"#));
    store.apply_db_row(DbRow::UiSettings, values(r#"{"disable_custom_api_keys": 2}"#));
    store.apply_db_row(row, values(r#"{"disable_custom_api_keys": 3, "pass_through_endpoints": 2}"#));
    assert_eq!(store.resolved(), values(r#"{"disable_custom_api_keys": 3, "pass_through_endpoints": 1}"#));
    assert_eq!(store.db_value("pass_through_endpoints"), None);
}

#[test]
fn pass_through_endpoints_come_from_the_db_only() {
    let mut store = general(
        r#"{"pass_through_endpoints": [{"path": "/file"}], "a": 1}"#,
        r#"{"pass_through_endpoints": [{"path": "/db"}], "a": 2}"#,
    );
    assert_eq!(store.resolution("pass_through_endpoints").source(), FieldSource::Db);
    assert_eq!(store.db_value("pass_through_endpoints"), Some(&json(r#"[{"path": "/db"}]"#)));
    assert_eq!(store.db_value("a"), None);
    assert_eq!(store.shadowed_db_keys(), vec![String::from("a")]);
    assert!(!store.shadows_db_value("pass_through_endpoints"));
    assert_eq!(store.insert("pass_through_endpoints", json("[]")), Ok(()));
    assert_eq!(store.get("pass_through_endpoints"), Some(&json("[]")));
}

#[test]
fn rejected_writes_name_changed_config_owned_keys() {
    let store = general(r#"{"a": 1, "b": 2, "c": 3}"#, "{}");
    let incoming = values(r#"{"a": 1, "b": 5, "c": 6, "free": 7}"#);
    assert_eq!(store.rejected_writes(&incoming), vec![String::from("b"), String::from("c")]);
}

#[rstest]
#[case::stored_differs("{}", r#"{"a": 2}"#, true)]
#[case::stored_matches_config("{}", r#"{"a": 1}"#, false)]
#[case::stored_null("{}", r#"{"a": null}"#, false)]
#[case::stored_missing("{}", "{}", false)]
#[case::stored_matches_runtime(r#"{"a": 2}"#, r#"{"a": 2}"#, false)]
#[case::config_matches_despite_runtime(r#"{"a": 7}"#, r#"{"a": 1}"#, false)]
fn shadowing_needs_a_different_stored_value(
    #[case] runtime: &str,
    #[case] stored: &str,
    #[case] shadows: bool,
) {
    let mut store = general(r#"{"a": 1}"#, stored);
    store.apply_runtime_values(values(runtime));
    assert_eq!(store.shadows_db_value("a"), shadows);
    assert_eq!(store.shadowed_db_keys().is_empty(), !shadows);
}

#[test]
fn clear_keeps_only_config_owned_keys() {
    let mut store = general(r#"{"a": 1}"#, r#"{"b": 2}"#);
    store.apply_runtime_values(values(r#"{"a": "resolved", "c": 3}"#));
    store.clear();
    assert_eq!(store.resolved(), values(r#"{"a": "resolved"}"#));
    store.apply_db_row(DbRow::GeneralSettings, values(r#"{"b": 4}"#));
    assert_eq!(store.resolved(), values(r#"{"a": "resolved", "b": 4}"#));
}
