#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::Settings;
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.proxy_provider_options)
        .map_err(|error| error.to_string())
}

fn loaded(source: &str, variables: &[(&str, &str)]) -> Settings {
    settings(source, variables).unwrap()
}

fn the_environment_supplies_the_base_and_key() {
    let settings = loaded(
        "model_list: []",
        &[
            ("LITELLM_PROXY_API_BASE", "https://proxy.example/v1"),
            ("LITELLM_PROXY_API_KEY", "sk-proxy"),
        ],
    );
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://proxy.example/v1")
    );
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-proxy")
    );
}

fn an_absent_environment_leaves_the_sources_false() {
    let settings = loaded("model_list: []", &[]);
    assert_eq!(settings.api_base, None);
    assert!(settings.api_key.is_none());
    assert!(!settings.environment_use_litellm_proxy);
    assert!(!settings.global_use_litellm_proxy);
    assert!(!settings.should_use_litellm_proxy(0));
}

fn a_boolean_spelling_stays_text_in_the_base_and_key() {
    let spellings = [
        ("LITELLM_PROXY_API_BASE", "true"),
        ("LITELLM_PROXY_API_KEY", "false"),
    ];
    let settings = loaded("model_list: []", &spellings);
    let base = settings.api_base.as_deref();
    let key = settings.api_key.as_ref().map(ExposeSecret::expose_secret);
    assert_eq!((base, key), (Some("true"), Some("false")));
}

fn an_empty_environment_counts_as_unset() {
    let settings = loaded(
        "model_list: []",
        &[
            ("LITELLM_PROXY_API_BASE", ""),
            ("LITELLM_PROXY_API_KEY", ""),
            ("USE_LITELLM_PROXY", ""),
        ],
    );
    assert_eq!(settings.api_base, None);
    assert!(settings.api_key.is_none());
    assert!(!settings.environment_use_litellm_proxy);
    assert!(!settings.should_use_litellm_proxy(0));
}

fn the_environment_flag_enables_forcing() {
    let settings = loaded(
        "model_list:\n  - model_name: proxy\n    litellm_params:\n      use_litellm_proxy: false",
        &[("USE_LITELLM_PROXY", "true")],
    );
    assert!(settings.should_use_litellm_proxy(0));
}

fn a_numeric_environment_flag_enables_forcing() {
    let settings = loaded("model_list: []", &[("USE_LITELLM_PROXY", "1")]);
    assert!(settings.environment_use_litellm_proxy);
    assert!(settings.should_use_litellm_proxy(0));
}

fn a_false_environment_flag_does_not_disable_a_deployment_flag() {
    let settings = loaded(
        "model_list:\n  - model_name: proxy\n    litellm_params:\n      use_litellm_proxy: true",
        &[("USE_LITELLM_PROXY", "false")],
    );
    assert!(settings.should_use_litellm_proxy(0));
    assert!(!settings.should_use_litellm_proxy(1));
}

fn the_global_setting_enables_forcing() {
    let settings = loaded(
        "litellm_settings:\n  use_litellm_proxy: true\nmodel_list:\n  - model_name: proxy\n    litellm_params:\n      use_litellm_proxy: false",
        &[("USE_LITELLM_PROXY", "false")],
    );
    assert!(settings.should_use_litellm_proxy(0));
}

fn a_deployment_flag_enables_forcing_for_that_deployment() {
    let settings = loaded(
        "model_list:\n  - model_name: forced\n    litellm_params:\n      model: openai/gpt-4o\n      use_litellm_proxy: true\n  - model_name: plain\n  - model_name: off\n    litellm_params:\n      model: openai/gpt-4o\n      use_litellm_proxy: false\n",
        &[],
    );
    assert!(settings.should_use_litellm_proxy(0));
    assert!(!settings.should_use_litellm_proxy(1));
    assert!(!settings.should_use_litellm_proxy(2));
    assert!(!settings.should_use_litellm_proxy(3));
}

fn a_null_deployment_flag_is_unset() {
    let settings = loaded(
        "model_list:\n  - model_name: proxy\n    litellm_params:\n      use_litellm_proxy: null",
        &[],
    );
    assert_eq!(settings.deployment_use_litellm_proxy, [false]);
    assert!(!settings.should_use_litellm_proxy(0));
}

fn a_non_flag_environment_value_names_the_variable() {
    let error = settings("model_list: []", &[("USE_LITELLM_PROXY", "maybe")]).unwrap_err();
    assert!(error.contains("USE_LITELLM_PROXY"), "{error}");
}

fn a_non_bool_global_setting_is_rejected() {
    let error = settings("litellm_settings:\n  use_litellm_proxy: maybe", &[]).unwrap_err();
    assert!(!error.is_empty(), "a string is not a bool");
}

fn a_non_bool_deployment_setting_is_rejected() {
    let source =
        "model_list:\n  - model_name: proxy\n    litellm_params:\n      use_litellm_proxy: 1.5\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty(), "a float is not a bool");
}

fn settings_debug_hides_the_key() {
    let settings = loaded(
        "model_list: []",
        &[("LITELLM_PROXY_API_KEY", "sk-proxy-secret")],
    );
    let debug = format!("{settings:?}");
    assert!(!debug.contains("sk-proxy-secret"), "{debug}");
    assert!(debug.contains("REDACTED"), "{debug}");
}

#[rstest]
#[case::the_environment_supplies_the_base_and_key(the_environment_supplies_the_base_and_key)]
#[case::an_absent_environment_leaves_the_sources_false(
    an_absent_environment_leaves_the_sources_false
)]
#[case::a_boolean_spelling_stays_text_in_the_base_and_key(
    a_boolean_spelling_stays_text_in_the_base_and_key
)]
#[case::an_empty_environment_counts_as_unset(an_empty_environment_counts_as_unset)]
#[case::the_environment_flag_enables_forcing(the_environment_flag_enables_forcing)]
#[case::a_numeric_environment_flag_enables_forcing(a_numeric_environment_flag_enables_forcing)]
#[case::a_false_environment_flag_does_not_disable_a_deployment_flag(
    a_false_environment_flag_does_not_disable_a_deployment_flag
)]
#[case::the_global_setting_enables_forcing(the_global_setting_enables_forcing)]
#[case::a_deployment_flag_enables_forcing_for_that_deployment(
    a_deployment_flag_enables_forcing_for_that_deployment
)]
#[case::a_null_deployment_flag_is_unset(a_null_deployment_flag_is_unset)]
#[case::a_non_flag_environment_value_names_the_variable(
    a_non_flag_environment_value_names_the_variable
)]
#[case::a_non_bool_global_setting_is_rejected(a_non_bool_global_setting_is_rejected)]
#[case::a_non_bool_deployment_setting_is_rejected(a_non_bool_deployment_setting_is_rejected)]
#[case::settings_debug_hides_the_key(settings_debug_hides_the_key)]
fn proxy_options_loads_sources(#[case] scenario: fn()) {
    scenario();
}
