#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

enum Scenario {
    EmptyLists,
    KnownName(&'static str),
    UnknownName,
    EveryList(&'static [&'static str]),
    SettingsScalars,
    Capacity(&'static [(&'static str, &'static str)], usize),
    NonList(&'static str),
    ScalarEntry(&'static str),
    NestedOwner,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.callback_settings)
}

#[rstest]
#[case::empty_lists_load(Scenario::EmptyLists)]
#[case::known_litellm_name_fails_startup(Scenario::KnownName("langfuse"))]
#[case::unknown_name_fails_startup(Scenario::UnknownName)]
#[case::every_unavailable_list_is_named(Scenario::EveryList(&[
    "litellm_settings.callbacks",
    "litellm_settings.success_callback",
    "litellm_settings.failure_callback",
]))]
#[case::callback_settings_scalars_become_text(Scenario::SettingsScalars)]
#[case::capacity_defaults_to_one_hundred(Scenario::Capacity(&[], 100))]
#[case::capacity_reads_the_environment(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "7")], 7))]
#[case::empty_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "")], 100))]
#[case::invalid_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "many")], 100))]
#[case::negative_capacity_admits_nothing(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "-1")], 0))]
#[case::non_list_callback_value_fails_as_yaml(Scenario::NonList(
    "litellm_settings:\n  callbacks: langfuse\n"
))]
#[case::non_string_callback_entry_reads_as_text(Scenario::ScalarEntry(
    "litellm_settings:\n  callbacks: [1]\n"
))]
#[case::nested_settings_value_owned_by_another_section(Scenario::NestedOwner)]
fn callback_registry_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::EmptyLists => {
            let settings = load(
                "litellm_settings:\n  callbacks: []\n  success_callback: []\n  failure_callback: []\n",
                &[],
            )
            .unwrap();
            assert_eq!(
                settings,
                Settings {
                    callbacks: Vec::new(),
                    success_callback: Vec::new(),
                    failure_callback: Vec::new(),
                    callback_settings: BTreeMap::new(),
                    max_callbacks: 100,
                }
            );
        }
        Scenario::KnownName(name) => {
            let source = format!("litellm_settings:\n  success_callback: [\"{name}\"]\n");
            let error = load(&source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["litellm_settings.success_callback"]));
        }
        Scenario::UnknownName => {
            let error = load(
                "litellm_settings:\n  callbacks: [\"not_a_litellm_callback\"]\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["litellm_settings.callbacks"]));
        }
        Scenario::EveryList(paths) => {
            let error = load(
                "litellm_settings:\n  callbacks: [langfuse]\n  success_callback: [langfuse]\n  failure_callback: [langfuse]\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths: found } if found == paths));
        }
        Scenario::SettingsScalars => {
            let settings = load(
                "callback_settings:\n  otel:\n    endpoint: https://collector\n    capture: true\n    retries: 3\n    empty: null\n    exporters: [{kind: console}]\n",
                &[],
            )
            .unwrap();
            let otel = settings
                .callback_settings
                .get("otel")
                .expect("otel settings are kept");
            assert_eq!(
                otel.get("endpoint").map(String::as_str),
                Some("https://collector")
            );
            assert_eq!(otel.get("capture").map(String::as_str), Some("True"));
            assert_eq!(otel.get("retries").map(String::as_str), Some("3"));
            assert!(!otel.contains_key("empty"));
            assert!(!otel.contains_key("exporters"));
            assert_eq!(settings.max_callbacks, 100);
        }
        Scenario::Capacity(environment, expected) => {
            let settings = load("", environment).unwrap();
            assert_eq!(settings.max_callbacks, expected);
        }
        Scenario::NonList(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Yaml { .. }));
        }
        Scenario::ScalarEntry(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["litellm_settings.callbacks"]));
        }
        Scenario::NestedOwner => {
            let settings = load(
                "callback_settings:\n  otel:\n    exporters:\n      - kind: console\n",
                &[],
            )
            .unwrap();
            assert!(settings.callback_settings.contains_key("otel"));
        }
    }
}
