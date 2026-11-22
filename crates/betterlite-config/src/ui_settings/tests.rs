#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{
    ALLOWED_UI_SETTINGS_FIELDS, BOOLEAN_UI_SETTINGS_FIELDS, PAGES_SETTING,
    PTU_COST_ATTRIBUTION_SETTING, USER_BUDGET_TEAM_KEYS_SETTING, field_default, load_ui_settings,
    ui_settings_from_store,
};
use crate::sections::settings::settings_resolution::{FieldSource, source_for};
use crate::sections::settings::settings_rules::{DbRow, JsonValue, Section};
use crate::sections::settings::settings_store::SettingsStore;
use crate::sections::team_admin_fields::EDITABLE_FIELDS_SETTING;

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn values(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

fn general_store(yaml: &str, stored: &str) -> SettingsStore {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(load_ui_settings(yaml).expect("the config loads"));
    store.apply_db_row(DbRow::UiSettings, values(stored));
    store
}

#[rstest]
#[case::disable_agents_for_internal_users("disable_agents_for_internal_users", "true", "true")]
#[case::allow_agents_for_team_admins("allow_agents_for_team_admins", "true", "true")]
#[case::require_auth_for_public_ai_hub("require_auth_for_public_ai_hub", "true", "true")]
#[case::allow_public_health_readiness_details(
    "allow_public_health_readiness_details",
    "true",
    "true"
)]
#[case::forward_client_headers_to_llm_api("forward_client_headers_to_llm_api", "true", "true")]
#[case::forward_llm_provider_auth_headers("forward_llm_provider_auth_headers", "true", "true")]
#[case::disable_vector_stores_for_internal_users(
    "disable_vector_stores_for_internal_users",
    "true",
    "true"
)]
#[case::allow_vector_stores_for_team_admins("allow_vector_stores_for_team_admins", "true", "true")]
#[case::scope_user_search_to_org("scope_user_search_to_org", "true", "true")]
#[case::disable_custom_api_keys("disable_custom_api_keys", "true", "true")]
#[case::disable_key_generate_for_org_admin("disable_key_generate_for_org_admin", "true", "true")]
#[case::enable_chat_ui("enable_chat_ui", "true", "true")]
#[case::disable_model_add_for_internal_users(
    "disable_model_add_for_internal_users",
    "true",
    "true"
)]
#[case::disable_team_admin_delete_team_user("disable_team_admin_delete_team_user", "true", "true")]
#[case::enabled_ui_pages_internal_users(
    "enabled_ui_pages_internal_users",
    r#"["chat", "models"]"#,
    r#"["chat","models"]"#
)]
#[case::team_admin_editable_team_fields(
    "team_admin_editable_team_fields",
    r#"["max_budget", "projects"]"#,
    r#"["max_budget","projects"]"#
)]
fn loads_ui_setting_flags_from_the_config_loader(
    #[case] key: &str,
    #[case] yaml_value: &str,
    #[case] expected: &str,
) {
    let source = format!("general_settings:\n  {key}: {yaml_value}\n");
    let settings = load_ui_settings(&source).expect("the config loads");
    assert_eq!(settings.get(key), Some(&json(expected)));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(settings);
    assert_eq!(store.get(key), Some(&json(expected)));
    assert_eq!(store.resolution(key).source(), FieldSource::Config);
}

#[rstest]
#[case::disable_agents_for_internal_users(
    "disable_agents_for_internal_users",
    "false",
    FieldSource::Default
)]
#[case::allow_agents_for_team_admins("allow_agents_for_team_admins", "false", FieldSource::Default)]
#[case::require_auth_for_public_ai_hub(
    "require_auth_for_public_ai_hub",
    "false",
    FieldSource::Default
)]
#[case::allow_public_health_readiness_details(
    "allow_public_health_readiness_details",
    "false",
    FieldSource::Default
)]
#[case::forward_client_headers_to_llm_api(
    "forward_client_headers_to_llm_api",
    "false",
    FieldSource::Default
)]
#[case::forward_llm_provider_auth_headers(
    "forward_llm_provider_auth_headers",
    "false",
    FieldSource::Default
)]
#[case::disable_vector_stores_for_internal_users(
    "disable_vector_stores_for_internal_users",
    "false",
    FieldSource::Default
)]
#[case::allow_vector_stores_for_team_admins(
    "allow_vector_stores_for_team_admins",
    "false",
    FieldSource::Default
)]
#[case::scope_user_search_to_org("scope_user_search_to_org", "false", FieldSource::Default)]
#[case::disable_custom_api_keys("disable_custom_api_keys", "false", FieldSource::Default)]
#[case::disable_key_generate_for_org_admin(
    "disable_key_generate_for_org_admin",
    "false",
    FieldSource::Default
)]
#[case::enable_chat_ui("enable_chat_ui", "false", FieldSource::Default)]
#[case::disable_model_add_for_internal_users(
    "disable_model_add_for_internal_users",
    "false",
    FieldSource::Default
)]
#[case::disable_team_admin_delete_team_user(
    "disable_team_admin_delete_team_user",
    "false",
    FieldSource::Default
)]
#[case::enabled_ui_pages_internal_users(
    "enabled_ui_pages_internal_users",
    "null",
    FieldSource::Unset
)]
#[case::team_admin_editable_team_fields(
    "team_admin_editable_team_fields",
    "[]",
    FieldSource::Default
)]
fn loads_ui_setting_flags_default_to_the_model_field_default(
    #[case] key: &str,
    #[case] expected: &str,
    #[case] source: FieldSource,
) {
    let settings = load_ui_settings("{}").expect("the config loads");
    assert_eq!(settings.get(key), None);
    let store = SettingsStore::from(Section::GeneralSettings);
    let default = field_default(key);
    assert_eq!(default, json(expected));
    assert_eq!(source_for(&store, key, &default), source);
}

#[test]
fn loads_ui_setting_flags_keep_config_and_db_provenance() {
    let mut store = general_store(
        "general_settings:\n  enable_chat_ui: true\n",
        r#"{"forward_client_headers_to_llm_api": true}"#,
    );
    store.apply_db_row(
        DbRow::GeneralSettings,
        values(r#"{"scope_user_search_to_org": false}"#),
    );
    assert_eq!(
        source_for(&store, "enable_chat_ui", &field_default("enable_chat_ui")),
        FieldSource::Config
    );
    assert_eq!(
        source_for(
            &store,
            "forward_client_headers_to_llm_api",
            &field_default("forward_client_headers_to_llm_api")
        ),
        FieldSource::Db
    );
    assert_eq!(
        source_for(
            &store,
            "require_auth_for_public_ai_hub",
            &field_default("require_auth_for_public_ai_hub")
        ),
        FieldSource::Default
    );
    assert_eq!(
        source_for(
            &store,
            "scope_user_search_to_org",
            &field_default("scope_user_search_to_org")
        ),
        FieldSource::Db
    );
}

#[test]
fn loads_ui_setting_flags_drop_keys_outside_the_allowlist() {
    let settings = load_ui_settings(
        "general_settings:\n  enable_chat_ui: true\n  master_key: sk-secret\n  enable_projects_ui: true\n",
    )
    .expect("the config loads");
    assert_eq!(settings, values(r#"{"enable_chat_ui": true}"#));
}

#[test]
fn loads_ui_setting_flags_keep_a_null_config_value_as_config() {
    let settings =
        load_ui_settings("general_settings:\n  enable_chat_ui: null\n").expect("the config loads");
    assert_eq!(settings.get("enable_chat_ui"), Some(&JsonValue::Null));
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(settings);
    assert_eq!(store.get("enable_chat_ui"), Some(&JsonValue::Null));
    assert_eq!(
        source_for(&store, "enable_chat_ui", &field_default("enable_chat_ui")),
        FieldSource::Config
    );
}

#[test]
fn loads_ui_setting_flags_read_the_effective_store_values() {
    let store = general_store(
        "general_settings:\n  enable_chat_ui: true\n",
        r#"{"forward_client_headers_to_llm_api": false, "enable_projects_ui": true}"#,
    );
    assert_eq!(
        ui_settings_from_store(&store),
        values(r#"{"enable_chat_ui": true, "forward_client_headers_to_llm_api": false}"#)
    );
}

#[test]
fn loads_ui_setting_flags_allow_exactly_the_sixteen_persisted_names() {
    assert_eq!(ALLOWED_UI_SETTINGS_FIELDS.len(), 16);
    assert_eq!(BOOLEAN_UI_SETTINGS_FIELDS.len(), 14);
    let unique: BTreeSet<&str> = ALLOWED_UI_SETTINGS_FIELDS.into_iter().collect();
    assert_eq!(unique.len(), 16);
    assert!(unique.contains(PAGES_SETTING));
    assert!(unique.contains(EDITABLE_FIELDS_SETTING));
    assert!(!unique.contains(PTU_COST_ATTRIBUTION_SETTING));
    assert!(!unique.contains(USER_BUDGET_TEAM_KEYS_SETTING));
    assert!(
        BOOLEAN_UI_SETTINGS_FIELDS
            .iter()
            .all(|name| unique.contains(name))
    );
}

#[test]
fn loads_ui_setting_flags_have_no_default_for_an_unknown_name() {
    assert_eq!(field_default("not_a_ui_setting"), JsonValue::Null);
}
