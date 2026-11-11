#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::health_check_settings::Settings;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.health_check_settings)
}

fn loaded(source: &str, environment: &[(&str, &str)]) -> Settings {
    load(source, environment).unwrap()
}

#[test]
fn an_empty_config_takes_the_litellm_defaults() {
    let settings = loaded("{}", &[]);
    assert!(!settings.background_health_checks);
    assert!(!settings.use_shared_health_check);
    assert_eq!(settings.health_check_interval, 300);
    assert!(settings.health_check_concurrency.is_none());
    assert!(settings.health_check_details);
}

#[rstest]
#[case::background_health_checks(
    "general_settings: {background_health_checks: true}",
    Settings {
        background_health_checks: true,
        use_shared_health_check: false,
        health_check_interval: 300,
        health_check_concurrency: None,
        health_check_details: true,
    }
)]
#[case::use_shared_health_check(
    "general_settings: {use_shared_health_check: true}",
    Settings {
        background_health_checks: false,
        use_shared_health_check: true,
        health_check_interval: 300,
        health_check_concurrency: None,
        health_check_details: true,
    }
)]
#[case::health_check_interval(
    "general_settings: {health_check_interval: 60}",
    Settings {
        background_health_checks: false,
        use_shared_health_check: false,
        health_check_interval: 60,
        health_check_concurrency: None,
        health_check_details: true,
    }
)]
#[case::health_check_concurrency(
    "general_settings: {health_check_concurrency: 4}",
    Settings {
        background_health_checks: false,
        use_shared_health_check: false,
        health_check_interval: 300,
        health_check_concurrency: Some(4),
        health_check_details: true,
    }
)]
#[case::health_check_details(
    "general_settings: {health_check_details: false}",
    Settings {
        background_health_checks: false,
        use_shared_health_check: false,
        health_check_interval: 300,
        health_check_concurrency: None,
        health_check_details: false,
    }
)]
fn each_key_alone_is_read(#[case] source: &str, #[case] expected: Settings) {
    assert_eq!(loaded(source, &[]), expected);
}

#[rstest]
#[case::background_health_checks("general_settings: {background_health_checks: null}")]
#[case::use_shared_health_check("general_settings: {use_shared_health_check: null}")]
#[case::health_check_interval("general_settings: {health_check_interval: null}")]
#[case::health_check_concurrency("general_settings: {health_check_concurrency: null}")]
#[case::health_check_details("general_settings: {health_check_details: null}")]
fn an_explicit_null_takes_the_default(#[case] source: &str) {
    assert_eq!(loaded(source, &[]), Settings::default());
}

#[rstest]
#[case::unset("{}", &[], 300)]
#[case::reads_the_environment("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "42")], 42)]
#[case::empty_is_unset("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "")], 300)]
#[case::blank_is_unset("{}", &[("DEFAULT_HEALTH_CHECK_INTERVAL", "  ")], 300)]
#[case::a_null_interval_uses_the_environment(
    "general_settings: {health_check_interval: null}",
    &[("DEFAULT_HEALTH_CHECK_INTERVAL", "42")],
    42
)]
#[case::a_configured_interval_wins(
    "general_settings: {health_check_interval: 60}",
    &[("DEFAULT_HEALTH_CHECK_INTERVAL", "10")],
    60
)]
fn default_health_check_interval_fills_a_missing_interval(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: i64,
) {
    assert_eq!(loaded(source, environment).health_check_interval, expected);
}

#[rstest]
#[case::not_a_number("{}", "abc")]
#[case::a_fraction("{}", "3.5")]
// LiteLLM parses the variable at import, so a configured interval does not spare a malformed value
#[case::a_configured_interval_does_not_spare_it(
    "general_settings: {health_check_interval: 60}",
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
#[case::background_health_checks(
    "general_settings: {background_health_checks: \"true\"}",
    "background_health_checks"
)]
#[case::use_shared_health_check(
    "general_settings: {use_shared_health_check: []}",
    "use_shared_health_check"
)]
#[case::health_check_interval(
    "general_settings: {health_check_interval: \"60\"}",
    "health_check_interval"
)]
#[case::a_fractional_interval(
    "general_settings: {health_check_interval: 60.5}",
    "health_check_interval"
)]
#[case::health_check_concurrency(
    "general_settings: {health_check_concurrency: \"4\"}",
    "health_check_concurrency"
)]
#[case::health_check_details(
    "general_settings: {health_check_details: \"yes\"}",
    "health_check_details"
)]
fn a_value_of_the_wrong_type_names_its_field(#[case] source: &str, #[case] field: &str) {
    let error = load(source, &[]).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidScalar { .. }));
    assert!(error.to_string().contains(field));
}

#[rstest]
#[case::zero("general_settings: {health_check_interval: 0}", 0)]
#[case::negative("general_settings: {health_check_interval: -5}", -5)]
fn a_non_positive_interval_stays_as_configured(#[case] source: &str, #[case] expected: i64) {
    assert_eq!(loaded(source, &[]).health_check_interval, expected);
}

#[rstest]
#[case::zero("general_settings: {health_check_concurrency: 0}", Some(0))]
#[case::negative("general_settings: {health_check_concurrency: -2}", Some(-2))]
fn a_non_positive_concurrency_stays_as_configured(
    #[case] source: &str,
    #[case] expected: Option<i64>,
) {
    assert_eq!(loaded(source, &[]).health_check_concurrency, expected);
}

#[test]
fn a_full_config_with_every_key_loads() {
    let source = "\
model_list:
  - model_name: gpt-4o-mini
    litellm_params:
      model: openai/gpt-4o-mini
      api_key: sk-health-check
general_settings:
  background_health_checks: True
  use_shared_health_check: true
  health_check_interval: 60
  health_check_concurrency: 2
  health_check_details: false
";
    assert_eq!(
        loaded(source, &[]),
        Settings {
            background_health_checks: true,
            use_shared_health_check: true,
            health_check_interval: 60,
            health_check_concurrency: Some(2),
            health_check_details: false,
        }
    );
}
