#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::health_probe_policy::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.health_probe_policy)
}

fn loaded(source: &str, environment: &[(&str, &str)]) -> Settings {
    load(source, environment).unwrap()
}

#[test]
fn an_empty_config_takes_the_litellm_defaults() {
    let settings = loaded("{}", &[]);
    assert!(!settings.skip_disabled_models);
    assert!(settings.model_groups.is_none());
    assert!(settings.max_tokens.is_none());
    assert!(settings.max_tokens_reasoning.is_none());
    assert_eq!(settings.timeout, Duration::from_secs(60));
    assert_eq!(settings.prompt, "test from litellm");
}

#[rstest]
#[case::absent("{}")]
#[case::explicit_false("general_settings: {health_check_skip_disabled_background_models: false}")]
fn the_skip_disabled_flag_defaults_to_false(#[case] source: &str) {
    assert!(!loaded(source, &[]).skip_disabled_models);
}

#[test]
fn the_skip_disabled_flag_reads_the_general_setting() {
    let settings = loaded(
        "general_settings: {health_check_skip_disabled_background_models: true}",
        &[],
    );
    assert!(settings.skip_disabled_models);
}

#[test]
fn a_malformed_skip_disabled_flag_fails_at_startup() {
    let error = load(
        "general_settings: {health_check_skip_disabled_background_models: sometimes}",
        &[],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.health_check_skip_disabled_background_models"
    ));
}

#[rstest]
#[case::absent("{}")]
#[case::explicit_null("general_settings: {background_health_check_model_groups: null}")]
fn unset_model_groups_cover_every_deployment(#[case] source: &str) {
    assert!(loaded(source, &[]).model_groups.is_none());
}

#[test]
fn an_empty_model_group_list_covers_no_deployment() {
    let settings = loaded(
        "general_settings: {background_health_check_model_groups: []}",
        &[],
    );
    assert_eq!(settings.model_groups, Some(BTreeSet::new()));
}

#[test]
fn model_groups_parse_into_a_set_with_duplicates_removed() {
    let settings = loaded(
        "general_settings:\n  background_health_check_model_groups: [gpt-4o, claude, gpt-4o]\n",
        &[],
    );
    let expected: BTreeSet<String> = ["gpt-4o", "claude"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(settings.model_groups, Some(expected));
}

#[rstest]
#[case::a_string("general_settings: {background_health_check_model_groups: gpt-4o}")]
#[case::a_number("general_settings: {background_health_check_model_groups: 4}")]
#[case::a_list_with_a_number(
    "general_settings: {background_health_check_model_groups: [gpt-4o, 4]}"
)]
fn malformed_model_groups_fail_at_startup(#[case] source: &str) {
    let error = load(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.background_health_check_model_groups"
    ));
}

#[rstest]
#[case::unset(&[])]
#[case::blank(&[("BACKGROUND_HEALTH_CHECK_MAX_TOKENS", "")])]
#[case::not_a_number(&[("BACKGROUND_HEALTH_CHECK_MAX_TOKENS", "many")])]
fn an_unreadable_token_limit_counts_as_unset(#[case] environment: &[(&str, &str)]) {
    assert!(loaded("{}", environment).max_tokens.is_none());
}

#[test]
fn the_token_limit_reads_the_environment() {
    let settings = loaded("{}", &[("BACKGROUND_HEALTH_CHECK_MAX_TOKENS", "32")]);
    assert_eq!(settings.max_tokens, Some(32));
}

#[test]
fn the_reasoning_token_limit_reads_its_own_environment() {
    let settings = loaded(
        "{}",
        &[
            ("BACKGROUND_HEALTH_CHECK_MAX_TOKENS", "32"),
            ("BACKGROUND_HEALTH_CHECK_MAX_TOKENS_REASONING", "64"),
        ],
    );
    assert_eq!(settings.max_tokens, Some(32));
    assert_eq!(settings.max_tokens_reasoning, Some(64));
}

#[test]
fn a_malformed_reasoning_token_limit_counts_as_unset() {
    let settings = loaded(
        "{}",
        &[("BACKGROUND_HEALTH_CHECK_MAX_TOKENS_REASONING", "lots")],
    );
    assert!(settings.max_tokens_reasoning.is_none());
}

#[rstest]
#[case::unset(&[], 60)]
#[case::blank(&[("HEALTH_CHECK_TIMEOUT_SECONDS", "")], 60)]
#[case::set(&[("HEALTH_CHECK_TIMEOUT_SECONDS", "5")], 5)]
#[case::zero(&[("HEALTH_CHECK_TIMEOUT_SECONDS", "0")], 0)]
fn the_timeout_reads_the_environment(#[case] environment: &[(&str, &str)], #[case] expected: u64) {
    assert_eq!(
        loaded("{}", environment).timeout,
        Duration::from_secs(expected)
    );
}

#[test]
fn a_malformed_timeout_fails_at_startup() {
    let error = load("{}", &[("HEALTH_CHECK_TIMEOUT_SECONDS", "soon")]).unwrap_err();
    assert!(matches!(error, ConfigError::Environment { .. }));
}

#[test]
fn a_negative_timeout_counts_as_zero() {
    let settings = loaded("{}", &[("HEALTH_CHECK_TIMEOUT_SECONDS", "-5")]);
    assert_eq!(settings.timeout, Duration::ZERO);
}

#[rstest]
#[case::unset(&[], "test from litellm")]
#[case::blank(&[("DEFAULT_HEALTH_CHECK_PROMPT", "")], "")]
#[case::set(&[("DEFAULT_HEALTH_CHECK_PROMPT", "ping")], "ping")]
fn the_prompt_reads_the_environment(#[case] environment: &[(&str, &str)], #[case] expected: &str) {
    assert_eq!(loaded("{}", environment).prompt, expected);
}
