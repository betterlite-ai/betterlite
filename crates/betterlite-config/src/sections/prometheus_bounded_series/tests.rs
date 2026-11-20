#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::prometheus_bounded_series::Settings;

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.prometheus_bounded_series)
}

fn load(source: &str) -> Settings {
    load_with(source, &[]).unwrap()
}

const MAX_SERIES: &str = "litellm_settings:\n  prometheus_end_user_metrics_max_series_per_metric: ";
const TTL: &str = "litellm_settings:\n  prometheus_end_user_metrics_ttl_seconds: ";
const CLEANUP: &str = "litellm_settings:\n  prometheus_end_user_metrics_cleanup_interval_seconds: ";
const MAX_SERIES_PATH: &str = "litellm_settings.prometheus_end_user_metrics_max_series_per_metric";
const TTL_PATH: &str = "litellm_settings.prometheus_end_user_metrics_ttl_seconds";
const CLEANUP_PATH: &str = "litellm_settings.prometheus_end_user_metrics_cleanup_interval_seconds";

#[test]
fn keeps_the_litellm_defaults_when_no_key_is_set() {
    let settings = load("model_list: []\n");
    assert_eq!(settings.max_series, Some(10000));
    assert_eq!(settings.ttl, Some(Duration::from_secs(3600)));
    assert_eq!(settings.cleanup_interval, Some(Duration::from_secs(60)));
    assert_eq!(settings, Settings::default());
}

#[test]
fn reads_a_configured_max_series() {
    let settings = load(&format!("{MAX_SERIES}500\n"));
    assert_eq!(settings.max_series, Some(500));
    assert_eq!(settings.ttl, Some(Duration::from_secs(3600)));
    assert_eq!(settings.cleanup_interval, Some(Duration::from_secs(60)));
}

#[test]
fn reads_a_configured_ttl_in_fractional_seconds() {
    let settings = load(&format!("{TTL}1.5\n"));
    assert_eq!(settings.max_series, Some(10000));
    assert_eq!(settings.ttl, Some(Duration::from_millis(1500)));
}

#[test]
fn reads_a_configured_cleanup_interval() {
    let settings = load(&format!("{CLEANUP}90\n"));
    assert_eq!(settings.cleanup_interval, Some(Duration::from_secs(90)));
}

#[test]
fn reads_a_configured_cleanup_interval_in_fractional_seconds() {
    let settings = load(&format!("{CLEANUP}0.5\n"));
    assert_eq!(settings.cleanup_interval, Some(Duration::from_millis(500)));
}

#[test]
fn treats_a_null_max_series_as_no_limit() {
    let settings = load(&format!("{MAX_SERIES}null\n"));
    assert_eq!(settings.max_series, None);
    assert_eq!(settings.ttl, Some(Duration::from_secs(3600)));
    assert_eq!(settings.cleanup_interval, Some(Duration::from_secs(60)));
}

#[test]
fn treats_a_null_ttl_as_no_limit() {
    let settings = load(&format!("{TTL}null\n"));
    assert_eq!(settings.max_series, Some(10000));
    assert_eq!(settings.ttl, None);
    assert_eq!(settings.cleanup_interval, Some(Duration::from_secs(60)));
}

#[test]
fn treats_a_null_cleanup_interval_as_no_interval() {
    let settings = load(&format!("{CLEANUP}null\n"));
    assert_eq!(settings.cleanup_interval, None);
}

#[test]
fn keeps_a_zero_or_negative_max_series_as_given() {
    assert_eq!(load(&format!("{MAX_SERIES}0\n")).max_series, Some(0));
    assert_eq!(load(&format!("{MAX_SERIES}-5\n")).max_series, Some(-5));
}

#[rstest]
#[case::zero("0")]
#[case::negative("-5")]
fn treats_a_nonpositive_cleanup_interval_as_no_interval(#[case] value: &str) {
    assert_eq!(load(&format!("{CLEANUP}{value}\n")).cleanup_interval, None);
}

#[rstest]
#[case::max_series(MAX_SERIES, "[5]", MAX_SERIES_PATH)]
#[case::ttl(TTL, "true", TTL_PATH)]
#[case::cleanup(CLEANUP, "[5]", CLEANUP_PATH)]
fn rejects_a_limit_of_another_type(#[case] prefix: &str, #[case] value: &str, #[case] path: &str) {
    let error = load_with(&format!("{prefix}{value}\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path: named, .. } if named == path
    ));
}

#[rstest]
#[case::text("words")]
#[case::fraction("1.5")]
fn rejects_a_max_series_that_is_not_an_integer(#[case] value: &str) {
    let error = load_with(&format!("{MAX_SERIES}{value}\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. } if path == MAX_SERIES_PATH
    ));
}

#[test]
fn rejects_a_ttl_that_is_not_a_number() {
    let error = load_with(&format!("{TTL}soon\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidFloat { path, .. } if path == TTL_PATH
    ));
}

#[test]
fn reads_an_environment_reference() {
    let settings = load_with(
        &format!("{MAX_SERIES}os.environ/MAX_SERIES\n"),
        &[("MAX_SERIES", "250")],
    )
    .unwrap();
    assert_eq!(settings.max_series, Some(250));
}

#[test]
fn treats_an_empty_environment_value_as_unset() {
    let settings = load_with(
        &format!("{MAX_SERIES}os.environ/MAX_SERIES\n"),
        &[("MAX_SERIES", "")],
    )
    .unwrap();
    assert_eq!(settings.max_series, Some(10000));
}

#[test]
fn reports_a_missing_environment_variable() {
    let error = load_with(&format!("{MAX_SERIES}os.environ/MAX_SERIES\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "MAX_SERIES" && path == MAX_SERIES_PATH
    ));
}
