#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

enum Scenario {
    EmptyLists,
    KnownName(&'static str),
    UnknownName,
    EveryList(&'static [&'static str]),
    SettingsScalars,
    SettingsDebugRedacts,
    Capacity(&'static [(&'static str, &'static str)], usize),
    StringCallbacks(&'static str),
    ScalarSuccessCallback(&'static str),
    ScalarEntry(&'static str),
    ResolvedSetting(&'static [(&'static str, &'static str)]),
    MissingSetting,
    NestedOwner,
    OtelUnknownScalar,
    OtelUnknownNested,
    NestedOwnerless,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.callback_settings)
}

fn variables(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
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
#[case::callback_settings_scalars_keep_their_text(Scenario::SettingsScalars)]
#[case::callback_settings_debug_redacts_credentials(Scenario::SettingsDebugRedacts)]
#[case::capacity_defaults_to_one_hundred(Scenario::Capacity(&[], 100))]
#[case::capacity_reads_the_environment(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "7")], 7))]
#[case::empty_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "")], 100))]
#[case::invalid_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "many")], 100))]
#[case::negative_capacity_admits_nothing(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "-1")], 0))]
#[case::single_string_callbacks_fail_startup(Scenario::StringCallbacks(
    "litellm_settings:\n  callbacks: custom_callbacks.proxy_handler_instance\n"
))]
#[case::scalar_success_callback_fails_as_yaml(Scenario::ScalarSuccessCallback(
    "litellm_settings:\n  success_callback: langfuse\n"
))]
#[case::non_string_callback_entry_fails_as_yaml(Scenario::ScalarEntry(
    "litellm_settings:\n  callbacks: [1]\n"
))]
#[case::callback_setting_resolves_an_environment_reference(Scenario::ResolvedSetting(&[
    ("CALLBACK_SECRET", "sk-resolved"),
]))]
#[case::callback_setting_missing_environment_variable_fails(Scenario::MissingSetting)]
#[case::nested_settings_value_owned_by_another_section(Scenario::NestedOwner)]
#[case::unknown_otel_scalar_is_kept(Scenario::OtelUnknownScalar)]
#[case::unknown_otel_nested_value_is_unsupported(Scenario::OtelUnknownNested)]
#[case::nested_settings_value_without_an_owner_is_unsupported(Scenario::NestedOwnerless)]
fn callback_registry_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::EmptyLists => {
            let settings = load(
                "litellm_settings:\n  callbacks: []\n  success_callback: []\n  failure_callback: []\n",
                &[],
            )
            .unwrap();
            assert!(settings.callbacks.is_empty());
            assert!(settings.success_callback.is_empty());
            assert!(settings.failure_callback.is_empty());
            assert!(settings.callback_settings.is_empty());
            assert_eq!(settings.max_callbacks, 100);
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
                "callback_settings:\n  langfuse:\n    endpoint: https://collector\n    capture: true\n    retries: 3\n    timeout: 1.5\n    optional: null\n",
                &[],
            )
            .unwrap();
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[
                    ("endpoint", "https://collector"),
                    ("capture", "True"),
                    ("retries", "3"),
                    ("timeout", "1.5"),
                ])
            );
            let debug = format!("{settings:?}");
            assert!(debug.contains("optional"));
            assert!(debug.contains("Null"));
        }
        Scenario::SettingsDebugRedacts => {
            let settings = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: sk-secret\n    headers: api-key=secret\n",
                &[],
            )
            .unwrap();
            let debug = format!("{settings:?}");
            assert!(!debug.contains("sk-secret"));
            assert!(!debug.contains("api-key=secret"));
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[
                    ("langfuse_secret", "sk-secret"),
                    ("headers", "api-key=secret"),
                ])
            );
        }
        Scenario::Capacity(environment, expected) => {
            let settings = load("", environment).unwrap();
            assert_eq!(settings.max_callbacks, expected);
        }
        Scenario::StringCallbacks(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["litellm_settings.callbacks"]));
        }
        Scenario::ScalarSuccessCallback(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Yaml { .. }));
        }
        Scenario::ResolvedSetting(environment) => {
            let settings = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: os.environ/CALLBACK_SECRET\n",
                environment,
            )
            .unwrap();
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[("langfuse_secret", "sk-resolved")])
            );
        }
        Scenario::MissingSetting => {
            let error = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: os.environ/CALLBACK_SECRET\n",
                &[],
            )
            .unwrap_err();
            assert!(
                matches!(error, ConfigError::MissingEnvironment { variable, path }
                if variable == "CALLBACK_SECRET" && path == "callback_settings.langfuse.langfuse_secret")
            );
        }
        Scenario::ScalarEntry(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Yaml { .. }));
        }
        Scenario::NestedOwner => {
            let settings = load(
                "callback_settings:\n  otel:\n    endpoint: https://collector\n    optional: null\n    exporters:\n      - kind: console\n",
                &[],
            )
            .unwrap();
            let otel = settings
                .callback_settings
                .get("otel")
                .expect("otel scalars are kept");
            assert_eq!(
                otel.exposed_variables(),
                variables(&[("endpoint", "https://collector")])
            );
        }
        Scenario::OtelUnknownScalar => {
            let settings = load(
                "callback_settings:\n  otel:\n    unknown_scalar: keep-me\n",
                &[],
            )
            .unwrap();
            let otel = settings
                .callback_settings
                .get("otel")
                .expect("otel scalars are kept");
            assert_eq!(
                otel.exposed_variables(),
                variables(&[("unknown_scalar", "keep-me")])
            );
        }
        Scenario::OtelUnknownNested => {
            let error = load(
                "callback_settings:\n  otel:\n    mystery:\n      depth: 1\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["callback_settings.otel.mystery"]));
        }
        Scenario::NestedOwnerless => {
            let error = load(
                "callback_settings:\n  custom:\n    event_types: [a]\n  generic_api:\n    endpoint: https://hook\n    headers:\n      authorization: Bearer secret\n",
                &[],
            )
            .unwrap_err();
            assert!(
                matches!(error, ConfigError::Unsupported { paths } if paths == [
                    "callback_settings.custom.event_types",
                    "callback_settings.generic_api.headers",
                ])
            );
        }
    }
}
