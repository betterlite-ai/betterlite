#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rstest::rstest;

use super::Settings;
use crate::loader::ConfigLoader;

const NAME: &str = "LITELLM_DISABLE_ADD_PREFIX_TO_PROMPT";

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.anthropic_response)
        .map_err(|error| error.to_string())
}

fn deployment_flag(flag: Option<bool>) -> &'static str {
    match flag {
        Some(true) => "      disable_add_prefix_to_prompt: true\n",
        Some(false) => "      disable_add_prefix_to_prompt: false\n",
        None => "",
    }
}

fn config(global: Option<bool>, deployments: &[Option<bool>]) -> String {
    let mut source = String::from("model_list:\n");
    if deployments.is_empty() {
        source.push_str("  []\n");
    }
    for (index, flag) in deployments.iter().enumerate() {
        write!(
            source,
            "  - model_name: m{index}\n    litellm_params:\n      model: anthropic/m{index}\n{}",
            deployment_flag(*flag)
        )
        .unwrap();
    }
    if let Some(flag) = global {
        writeln!(
            source,
            "litellm_settings:\n  disable_add_prefix_to_prompt: {flag}"
        )
        .unwrap();
    }
    source
}

fn explicit_values_load() {
    let enabled = settings(&config(Some(true), &[]), &[]).unwrap();
    assert!(enabled.inner.global_disable_prefix);
    assert!(enabled.inner.deployment_disable_prefix.is_empty());
    let disabled = settings(&config(Some(false), &[]), &[]).unwrap();
    assert!(!disabled.inner.global_disable_prefix);
}

fn an_absent_flag_defaults_to_false() {
    let defaults = settings(&config(None, &[]), &[]).unwrap();
    assert_eq!(defaults, Settings::default());
    let null = settings(
        "litellm_settings:\n  disable_add_prefix_to_prompt: null\n",
        &[],
    )
    .unwrap();
    assert!(!null.inner.global_disable_prefix);
}

fn deployments_fall_back_to_the_global_flag() {
    let resolved = settings(&config(Some(true), &[None, Some(false), Some(true)]), &[]).unwrap();
    assert!(resolved.inner.global_disable_prefix);
    assert_eq!(
        resolved.inner.deployment_disable_prefix,
        [true, false, true]
    );
}

fn the_environment_enables_the_global_flag() {
    let enabled = settings(&config(None, &[]), &[(NAME, "yes")]).unwrap();
    assert!(enabled.inner.global_disable_prefix);
    let deployed = settings(&config(None, &[None]), &[(NAME, "1")]).unwrap();
    assert_eq!(deployed.inner.deployment_disable_prefix, [true]);
}

fn an_explicit_flag_overrides_the_environment() {
    let resolved = settings(&config(Some(false), &[Some(false)]), &[(NAME, "true")]).unwrap();
    assert!(!resolved.inner.global_disable_prefix);
    assert_eq!(resolved.inner.deployment_disable_prefix, [false]);
}

fn an_empty_environment_is_unset() {
    let resolved = settings(&config(None, &[]), &[(NAME, "")]).unwrap();
    assert!(!resolved.inner.global_disable_prefix);
}

fn a_non_flag_environment_names_the_variable() {
    let error = settings(&config(None, &[]), &[(NAME, "always")]).unwrap_err();
    assert!(error.contains(NAME), "{error}");
    assert!(error.contains("must be a bool"), "{error}");
}

fn unrelated_keys_are_rejected() {
    let deployment = "model_list:\n  - model_name: a\n    litellm_params:\n      model: anthropic/a\n      disable_add_prefix_to_prompt_extra: true\n";
    let error = settings(deployment, &[]).unwrap_err();
    assert!(
        error.contains("model_list.0.litellm_params.disable_add_prefix_to_prompt_extra"),
        "{error}"
    );
    let global = "litellm_settings:\n  disable_add_prefix_to_prompt_extra: true\n";
    let global_error = settings(global, &[]).unwrap_err();
    assert!(
        global_error.contains("litellm_settings.disable_add_prefix_to_prompt_extra"),
        "{global_error}"
    );
}

fn loads_disable_add_prefix_to_prompt() {
    explicit_values_load();
    an_absent_flag_defaults_to_false();
    deployments_fall_back_to_the_global_flag();
    the_environment_enables_the_global_flag();
    an_explicit_flag_overrides_the_environment();
    an_empty_environment_is_unset();
    a_non_flag_environment_names_the_variable();
    unrelated_keys_are_rejected();
}

#[test]
fn loads_the_anthropic_reply_corpus_seeds() {
    let valid = include_str!("../../../fuzz/corpus/config_yaml/anthropic_reply-valid.yaml");
    let seed = settings(valid, &[]).unwrap();
    assert!(seed.inner.global_disable_prefix);
    assert_eq!(seed.inner.deployment_disable_prefix, [false, true]);
    let unrelated = include_str!("../../../fuzz/corpus/config_yaml/anthropic_reply-unrelated.yaml");
    let error = settings(unrelated, &[]).unwrap_err();
    assert!(
        error.contains("model_list.0.litellm_params.anthropic_reply_extra"),
        "{error}"
    );
}

#[rstest]
#[case::loads_disable_add_prefix_to_prompt(loads_disable_add_prefix_to_prompt)]
#[case::explicit_values_load(explicit_values_load)]
#[case::an_absent_flag_defaults_to_false(an_absent_flag_defaults_to_false)]
#[case::deployments_fall_back_to_the_global_flag(deployments_fall_back_to_the_global_flag)]
#[case::the_environment_enables_the_global_flag(the_environment_enables_the_global_flag)]
#[case::an_explicit_flag_overrides_the_environment(an_explicit_flag_overrides_the_environment)]
#[case::an_empty_environment_is_unset(an_empty_environment_is_unset)]
#[case::a_non_flag_environment_names_the_variable(a_non_flag_environment_names_the_variable)]
#[case::unrelated_keys_are_rejected(unrelated_keys_are_rejected)]
fn anthropic_reply_config_cases(#[case] scenario: fn()) {
    scenario();
}
