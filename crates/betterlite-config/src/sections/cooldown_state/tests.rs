#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::cooldown_state::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.cooldown_state)
}

fn loaded(source: &str, environment: &[(&str, &str)]) -> Settings {
    load(source, environment).unwrap()
}

#[test]
fn an_empty_config_takes_the_litellm_defaults() {
    let settings = loaded("{}", &[]);
    assert!(!settings.enable_health_check_routing);
    assert_eq!(settings.health_check_staleness_threshold, 600.0);
    assert!(!settings.health_check_ignore_transient_errors);
    assert_eq!(settings.cooldown_redis_read_interval_seconds, 1.0);
}

#[rstest]
#[case::enable_health_check_routing(
    "general_settings: {enable_health_check_routing: true}",
    &[],
    Settings {
        enable_health_check_routing: true,
        health_check_staleness_threshold: 600.0,
        health_check_ignore_transient_errors: false,
        cooldown_redis_read_interval_seconds: 1.0,
    }
)]
#[case::health_check_staleness_threshold(
    "general_settings: {health_check_staleness_threshold: 90}",
    &[],
    Settings {
        enable_health_check_routing: false,
        health_check_staleness_threshold: 90.0,
        health_check_ignore_transient_errors: false,
        cooldown_redis_read_interval_seconds: 1.0,
    }
)]
#[case::a_fractional_threshold(
    "general_settings: {health_check_staleness_threshold: 2.5}",
    &[],
    Settings {
        enable_health_check_routing: false,
        health_check_staleness_threshold: 2.5,
        health_check_ignore_transient_errors: false,
        cooldown_redis_read_interval_seconds: 1.0,
    }
)]
#[case::a_threshold_from_the_environment(
    "general_settings: {health_check_staleness_threshold: os.environ/STALENESS_SECONDS}",
    &[("STALENESS_SECONDS", "45")],
    Settings {
        enable_health_check_routing: false,
        health_check_staleness_threshold: 45.0,
        health_check_ignore_transient_errors: false,
        cooldown_redis_read_interval_seconds: 1.0,
    }
)]
#[case::health_check_ignore_transient_errors(
    "general_settings: {health_check_ignore_transient_errors: true}",
    &[],
    Settings {
        enable_health_check_routing: false,
        health_check_staleness_threshold: 600.0,
        health_check_ignore_transient_errors: true,
        cooldown_redis_read_interval_seconds: 1.0,
    }
)]
fn each_key_alone_is_read(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Settings,
) {
    assert_eq!(loaded(source, environment), expected);
}

#[rstest]
#[case::enable_health_check_routing("general_settings: {enable_health_check_routing: null}")]
#[case::health_check_staleness_threshold(
    "general_settings: {health_check_staleness_threshold: null}"
)]
#[case::health_check_ignore_transient_errors(
    "general_settings: {health_check_ignore_transient_errors: null}"
)]
fn an_explicit_null_takes_the_default(#[case] source: &str) {
    assert_eq!(loaded(source, &[]), Settings::default());
}

#[rstest]
#[case::unset("{}", &[], 600.0)]
#[case::reads_the_environment("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "42")], 84.0)]
#[case::empty_is_unset("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "")], 600.0)]
#[case::blank_is_unset("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "  ")], 600.0)]
#[case::a_configured_threshold_wins(
    "general_settings: {health_check_staleness_threshold: 5}",
    &[("DEFAULT_HEALTH_CHECK_INTERVAL", "10")],
    5.0
)]
fn the_staleness_threshold_defaults_to_twice_the_health_check_interval(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: f64,
) {
    assert_eq!(
        loaded(source, environment).health_check_staleness_threshold,
        expected
    );
}

#[rstest]
#[case::not_a_number("{}", "abc")]
// LiteLLM parses the variable at import, so a configured threshold does not spare a malformed value
#[case::a_configured_threshold_does_not_spare_it(
    "general_settings: {health_check_staleness_threshold: 5}",
    "abc"
)]
fn a_malformed_default_health_check_interval_names_the_variable(
    #[case] source: &str,
    #[case] value: &str,
) {
    let error = load(source, &[("DEFAULT_HEALTH_CHECK_INTERVAL", value)]).unwrap_err();
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains("DEFAULT_HEALTH_CHECK_INTERVAL"));
}

#[rstest]
#[case::unset("{}", &[], 1.0)]
#[case::reads_the_environment("{}", &[("DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS", "0.25")], 0.25)]
#[case::empty_is_unset("{}", &[("DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS", "")], 1.0)]
#[case::blank_is_unset("{}", &[("DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS", "  ")], 1.0)]
fn the_redis_read_interval_defaults_to_one_second(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: f64,
) {
    assert_eq!(
        loaded(source, environment).cooldown_redis_read_interval_seconds,
        expected
    );
}

#[rstest]
#[case::not_a_number("abc")]
#[case::a_colon("12:30")]
fn a_malformed_redis_read_interval_names_the_variable(#[case] value: &str) {
    let error = load(
        "{}",
        &[("DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS", value)],
    )
    .unwrap_err();
    assert_eq!(error.category(), "environment");
    assert!(
        error
            .to_string()
            .contains("DEFAULT_COOLDOWN_REDIS_READ_INTERVAL_SECONDS")
    );
}

#[rstest]
#[case::enable_health_check_routing("general_settings: {enable_health_check_routing: \"true\"}")]
#[case::health_check_staleness_threshold(
    "general_settings: {health_check_staleness_threshold: true}"
)]
#[case::health_check_ignore_transient_errors(
    "general_settings: {health_check_ignore_transient_errors: []}"
)]
fn a_wrong_typed_key_names_its_path(#[case] source: &str) {
    let error = load(source, &[]).unwrap_err();
    assert_eq!(error.category(), "invalid_scalar");
    assert!(error.to_string().contains("general_settings."));
}
