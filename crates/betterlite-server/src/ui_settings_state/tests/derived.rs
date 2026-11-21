use std::collections::BTreeMap;

use betterlite_config::ptu_pricing::PTU_COST_ATTRIBUTION_ENV_VAR;
use betterlite_config::sections::settings::settings_resolution::FieldSource;
use betterlite_config::sections::settings::settings_rules::{DbRow, Section};
use betterlite_config::sections::settings::settings_store::SettingsStore;
use betterlite_config::ui_settings::{PTU_COST_ATTRIBUTION_SETTING, USER_BUDGET_TEAM_KEYS_SETTING};
use rstest::rstest;
use secrecy::SecretString;

use super::support::{FakePtuFlag, json, settings};
use crate::jobs::ptu::attribution::EnvironmentAttribution;
use crate::ui_settings_state::derived::{
    DerivedUiSettings, PtuCostFlag, derived_ui_settings, ui_setting_source,
};

fn store() -> SettingsStore {
    SettingsStore::from(Section::GeneralSettings)
}

#[test]
fn derived_flags_read_the_ptu_service_and_the_exact_true_budget() {
    let ptu = FakePtuFlag {
        enabled: true,
        configured: false,
    };
    let derived = derived_ui_settings(
        &ptu,
        &settings(r#"{"apply_user_budget_to_team_keys": true}"#),
    );
    assert_eq!(
        derived,
        DerivedUiSettings {
            enable_ptu_cost_attribution: true,
            apply_user_budget_to_team_keys: true,
        }
    );
}

#[rstest]
#[case::missing("{}", false)]
#[case::flag_false(r#"{"apply_user_budget_to_team_keys": false}"#, false)]
#[case::integer_one(r#"{"apply_user_budget_to_team_keys": 1}"#, false)]
#[case::text_true(r#"{"apply_user_budget_to_team_keys": "true"}"#, false)]
#[case::list(r#"{"apply_user_budget_to_team_keys": [true]}"#, false)]
fn derived_budget_flag_requires_the_exact_boolean_true(
    #[case] general_settings: &str,
    #[case] expected: bool,
) {
    let ptu = FakePtuFlag {
        enabled: false,
        configured: false,
    };
    assert_eq!(
        derived_ui_settings(&ptu, &settings(general_settings)).apply_user_budget_to_team_keys,
        expected
    );
}

#[rstest]
#[case::configured("false", true, "false", FieldSource::Config)]
#[case::value_true("false", false, "true", FieldSource::Config)]
#[case::unconfigured("false", false, "false", FieldSource::Default)]
fn ptu_source_is_config_when_configured_or_true(
    #[case] value: &str,
    #[case] configured: bool,
    #[case] resolved: &str,
    #[case] source: FieldSource,
) {
    let ptu = FakePtuFlag {
        enabled: resolved == "true",
        configured,
    };
    assert_eq!(
        ui_setting_source(PTU_COST_ATTRIBUTION_SETTING, &json(value), &store(), &ptu),
        source
    );
}

#[rstest]
#[case::flag_true("true", FieldSource::Config)]
#[case::flag_false("false", FieldSource::Default)]
#[case::integer_one("1", FieldSource::Default)]
#[case::text_true(r#""true""#, FieldSource::Default)]
fn budget_source_is_config_only_for_the_exact_true(
    #[case] value: &str,
    #[case] source: FieldSource,
) {
    let ptu = FakePtuFlag {
        enabled: false,
        configured: false,
    };
    assert_eq!(
        ui_setting_source(USER_BUDGET_TEAM_KEYS_SETTING, &json(value), &store(), &ptu),
        source
    );
}

#[test]
fn ordinary_sources_follow_the_settings_store() {
    let mut store = SettingsStore::from(Section::GeneralSettings);
    store.load_yaml(settings(r#"{"enable_chat_ui": true}"#));
    store.apply_db_row(
        DbRow::UiSettings,
        settings(r#"{"forward_client_headers_to_llm_api": true}"#),
    );
    let ptu = FakePtuFlag {
        enabled: false,
        configured: false,
    };
    assert_eq!(
        ui_setting_source("enable_chat_ui", &json("true"), &store, &ptu),
        FieldSource::Config
    );
    assert_eq!(
        ui_setting_source(
            "forward_client_headers_to_llm_api",
            &json("true"),
            &store,
            &ptu
        ),
        FieldSource::Db
    );
    assert_eq!(
        ui_setting_source(
            "require_auth_for_public_ai_hub",
            &json("false"),
            &store,
            &ptu
        ),
        FieldSource::Default
    );
    assert_eq!(
        ui_setting_source(
            "enabled_ui_pages_internal_users",
            &json("null"),
            &store,
            &ptu
        ),
        FieldSource::Unset
    );
    assert_eq!(
        ui_setting_source("not_a_ui_setting", &json("null"), &store, &ptu),
        FieldSource::Unset
    );
}

#[test]
fn environment_attribution_reports_a_configured_environment_value() {
    let configured = BTreeMap::from([(
        PTU_COST_ATTRIBUTION_ENV_VAR.to_owned(),
        Some(SecretString::from("false".to_owned())),
    )]);
    let ptu = EnvironmentAttribution { configured };
    assert!(ptu.configured());
    assert!(!ptu.enabled());
    assert_eq!(
        ui_setting_source(PTU_COST_ATTRIBUTION_SETTING, &json("false"), &store(), &ptu),
        FieldSource::Config
    );
}

#[test]
fn environment_attribution_derives_the_enabled_flag() {
    let configured = BTreeMap::from([(
        PTU_COST_ATTRIBUTION_ENV_VAR.to_owned(),
        Some(SecretString::from("true".to_owned())),
    )]);
    let ptu = EnvironmentAttribution { configured };
    assert!(ptu.enabled());
    assert!(derived_ui_settings(&ptu, &settings("{}")).enable_ptu_cost_attribution);
}
