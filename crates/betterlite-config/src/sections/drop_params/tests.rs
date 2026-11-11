#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{DeploymentDropPolicy, Settings};
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.drop_params)
        .map_err(|error| error.to_string())
}

fn deployment(drop_params: Option<bool>, additional: &[&str]) -> DeploymentDropPolicy {
    DeploymentDropPolicy {
        drop_params,
        additional_drop_params: additional.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn absent_config_has_no_drop_policy() {
    let settings = settings("model_list: []", &[]).unwrap();
    assert!(!settings.policy.global_drop_params);
    assert!(settings.policy.deployments.is_empty());
}

fn litellm_settings_enables_the_global_flag() {
    let settings = settings("litellm_settings:\n  drop_params: true", &[]).unwrap();
    assert!(settings.policy.global_drop_params);
}

fn an_explicit_false_overrides_the_environment() {
    let settings = settings(
        "litellm_settings:\n  drop_params: false",
        &[("LITELLM_DROP_PARAMS", "true")],
    )
    .unwrap();
    assert!(!settings.policy.global_drop_params);
}

fn the_environment_enables_the_global_flag() {
    let settings = settings("model_list: []", &[("LITELLM_DROP_PARAMS", "yes")]).unwrap();
    assert!(settings.policy.global_drop_params);
}

fn an_empty_environment_is_unset() {
    let settings = settings("model_list: []", &[("LITELLM_DROP_PARAMS", "")]).unwrap();
    assert!(!settings.policy.global_drop_params);
}

fn a_non_flag_environment_turns_the_global_flag_on() {
    let settings = settings("model_list: []", &[("LITELLM_DROP_PARAMS", "always")]).unwrap();
    assert!(settings.policy.global_drop_params);
}

fn a_non_list_additional_drop_params_fails() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      additional_drop_params: {temperature: true}\n";
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty(), "a map is not a drop list");
}

fn deployment_policies_stay_aligned_with_model_list() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      drop_params: true\n      additional_drop_params: [temperature, logprobs]\n  - model_name: b\n    litellm_params:\n      model: openai/gpt-4o\n";
    let settings = settings(source, &[]).unwrap();
    assert_eq!(
        settings.policy.deployments.as_ref(),
        [
            deployment(Some(true), &["temperature", "logprobs"]),
            deployment(None, &[]),
        ]
    );
}

fn a_null_deployment_flag_is_unset() {
    let source = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      drop_params: null\n      additional_drop_params: []\n";
    let settings = settings(source, &[]).unwrap();
    assert_eq!(
        settings.policy.deployments.as_ref(),
        [deployment(None, &[])]
    );
}

fn a_nested_drop_path_is_rejected() {
    let dotted = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      additional_drop_params: ['tools.function']\n";
    let error = settings(dotted, &[]).unwrap_err();
    assert!(
        error.contains("model_list.0.litellm_params.additional_drop_params[tools.function]"),
        "{error}"
    );
    let indexed = "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      additional_drop_params: ['tools[0]']\n";
    let indexed_error = settings(indexed, &[]).unwrap_err();
    assert!(
        indexed_error.contains("model_list.0.litellm_params.additional_drop_params[tools[0]]"),
        "{indexed_error}"
    );
}

#[rstest]
#[case::a_nested_drop_path_is_rejected(a_nested_drop_path_is_rejected)]
#[case::absent_config_has_no_drop_policy(absent_config_has_no_drop_policy)]
#[case::litellm_settings_enables_the_global_flag(litellm_settings_enables_the_global_flag)]
#[case::an_explicit_false_overrides_the_environment(an_explicit_false_overrides_the_environment)]
#[case::the_environment_enables_the_global_flag(the_environment_enables_the_global_flag)]
#[case::an_empty_environment_is_unset(an_empty_environment_is_unset)]
#[case::a_non_flag_environment_turns_the_global_flag_on(
    a_non_flag_environment_turns_the_global_flag_on
)]
#[case::a_non_list_additional_drop_params_fails(a_non_list_additional_drop_params_fails)]
#[case::deployment_policies_stay_aligned_with_model_list(
    deployment_policies_stay_aligned_with_model_list
)]
#[case::a_null_deployment_flag_is_unset(a_null_deployment_flag_is_unset)]
fn provider_params_tests_behavior(#[case] scenario: fn()) {
    scenario();
}
