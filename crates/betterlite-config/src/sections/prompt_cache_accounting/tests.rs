#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::prompt_cache_accounting::{CacheTtl, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.prompt_cache_accounting
}

fn source(value: &str) -> String {
    format!("litellm_settings:\n  anthropic_prompt_caching_ttl: {value}")
}

#[rstest]
#[case::five_minutes("5m", CacheTtl::FiveMinutes)]
#[case::one_hour("1h", CacheTtl::OneHour)]
#[case::quoted("'1h'", CacheTtl::OneHour)]
fn valid_ttls_are_kept(#[case] value: &str, #[case] expected: CacheTtl) {
    let loaded = settings(&source(value), &[]);
    assert_eq!(loaded.ttl, Some(expected));
}

#[rstest]
#[case::other_spelling("2h")]
#[case::empty_text("''")]
#[case::null("")]
#[case::null_literal("null")]
#[case::long_lived("30d")]
#[case::upper_case("1H")]
fn invalid_or_unset_ttls_leave_the_default(#[case] value: &str) {
    let loaded = settings(&source(value), &[]);
    assert_eq!(loaded.ttl, None);
}

#[test]
fn an_absent_key_leaves_the_default() {
    let loaded = settings("litellm_settings: {}", &[]);
    assert_eq!(loaded.ttl, None);
}

#[test]
fn an_absent_key_reads_the_environment_variable() {
    let loaded = settings(
        "litellm_settings: {}",
        &[("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", "1h")],
    );
    assert_eq!(loaded.ttl, Some(CacheTtl::OneHour));
}

#[test]
fn the_config_value_overrides_the_environment_variable() {
    let loaded = settings(
        &source("5m"),
        &[("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", "1h")],
    );
    assert_eq!(loaded.ttl, Some(CacheTtl::FiveMinutes));
}

#[rstest]
#[case::empty_variable("")]
#[case::invalid_variable("2h")]
fn an_unusable_environment_variable_leaves_the_default(#[case] value: &str) {
    let loaded = settings(
        "litellm_settings: {}",
        &[("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", value)],
    );
    assert_eq!(loaded.ttl, None);
}

#[test]
fn an_environment_reference_is_resolved() {
    let loaded = settings(&source("os.environ/TTL"), &[("TTL", "1h")]);
    assert_eq!(loaded.ttl, Some(CacheTtl::OneHour));
}

#[test]
fn a_missing_environment_reference_is_a_load_error() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load(&source("os.environ/TTL"))
        .expect_err("a missing reference is a load error");
    assert!(error.to_string().contains("TTL"));
}

#[test]
fn a_non_text_scalar_leaves_the_default() {
    let loaded = settings(&source("true"), &[]);
    assert_eq!(loaded.ttl, None);
}

#[test]
fn the_ttl_setting_is_no_longer_an_unsupported_key() {
    let loaded = settings(&source("1h"), &[]);
    assert_eq!(loaded.ttl, Some(CacheTtl::OneHour));
}

#[rstest]
#[case::five_minutes(CacheTtl::FiveMinutes, "5m")]
#[case::one_hour(CacheTtl::OneHour, "1h")]
fn the_policy_renders_the_wire_spelling(#[case] ttl: CacheTtl, #[case] expected: &str) {
    assert_eq!(ttl.as_str(), expected);
}
