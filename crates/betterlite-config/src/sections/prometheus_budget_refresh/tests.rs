#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{DEFAULT_INTERVAL_MINUTES, INTERVAL_VARIABLE, Settings};

const PATH: &str = "litellm_settings.prometheus_initialize_budget_metrics";

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.prometheus_budget_refresh)
}

fn toggle_source(value: &str) -> String {
    format!("model_list: []\nlitellm_settings:\n  prometheus_initialize_budget_metrics: {value}\n")
}

#[test]
fn defaults_disable_the_refresh_and_use_five_minutes() {
    let loaded = load("model_list: []\n", &[]).unwrap();

    assert_eq!(
        loaded,
        Settings {
            initialize_budget_metrics: false,
            refresh_interval_minutes: DEFAULT_INTERVAL_MINUTES,
        }
    );
    assert_eq!(DEFAULT_INTERVAL_MINUTES, 5);
}

#[test]
fn the_literal_true_enables_the_refresh() {
    let loaded = load(&toggle_source("true"), &[]).unwrap();

    assert!(loaded.initialize_budget_metrics);
}

#[test]
fn the_literal_false_keeps_it_off() {
    let loaded = load(&toggle_source("false"), &[]).unwrap();

    assert!(!loaded.initialize_budget_metrics);
}

#[test]
fn an_environment_reference_reads_the_flag() {
    let loaded = load(
        &toggle_source("os.environ/PROMETHEUS_BUDGET_REFRESH_ENABLED"),
        &[("PROMETHEUS_BUDGET_REFRESH_ENABLED", "on")],
    )
    .unwrap();

    assert!(loaded.initialize_budget_metrics);
}

#[test]
fn an_empty_environment_flag_counts_as_unset() {
    let loaded = load(
        &toggle_source("os.environ/PROMETHEUS_BUDGET_REFRESH_ENABLED"),
        &[("PROMETHEUS_BUDGET_REFRESH_ENABLED", "")],
    )
    .unwrap();

    assert!(!loaded.initialize_budget_metrics);
}

#[test]
fn a_missing_environment_flag_names_the_variable() {
    let error = load(
        &toggle_source("os.environ/PROMETHEUS_BUDGET_REFRESH_ENABLED"),
        &[],
    )
    .expect_err("the variable is not set");

    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "PROMETHEUS_BUDGET_REFRESH_ENABLED" && path == PATH
    ));
}

#[test]
fn a_non_boolean_environment_flag_names_the_variable() {
    let error = load(
        &toggle_source("os.environ/PROMETHEUS_BUDGET_REFRESH_ENABLED"),
        &[("PROMETHEUS_BUDGET_REFRESH_ENABLED", "maybe")],
    )
    .expect_err("a word is not a flag");

    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Bool { name },
        } if name == "PROMETHEUS_BUDGET_REFRESH_ENABLED"
    ));
}

#[test]
fn a_literal_string_is_not_a_boolean() {
    let error = load(&toggle_source("\"yes\""), &[]).expect_err("a string is not a bool");

    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == PATH
    ));
}

#[test]
fn a_number_is_not_a_boolean() {
    let error = load(&toggle_source("1"), &[]).expect_err("a number is not a bool");

    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == PATH
    ));
}

#[rstest]
#[case::ten(Some("10"), 10)]
#[case::empty(Some(""), DEFAULT_INTERVAL_MINUTES)]
#[case::unset(None, DEFAULT_INTERVAL_MINUTES)]
fn resolves_the_interval(#[case] text: Option<&str>, #[case] expected: i64) {
    let variables: Vec<(&str, &str)> = text
        .map(|raw| (INTERVAL_VARIABLE, raw))
        .into_iter()
        .collect();
    let loaded = load("model_list: []\n", &variables).unwrap();

    assert_eq!(loaded.refresh_interval_minutes, expected);
}

#[test]
fn a_word_in_the_interval_names_the_variable() {
    let error = load("model_list: []\n", &[(INTERVAL_VARIABLE, "abc")])
        .expect_err("a word is not a number");

    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == INTERVAL_VARIABLE
    ));
}

#[test]
fn a_fractional_interval_names_the_variable() {
    let error = load("model_list: []\n", &[(INTERVAL_VARIABLE, "2.5")])
        .expect_err("minutes are whole numbers");

    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == INTERVAL_VARIABLE
    ));
}

#[test]
fn an_overflowing_interval_names_the_variable() {
    let error = load(
        "model_list: []\n",
        &[(INTERVAL_VARIABLE, "9223372036854775808")],
    )
    .expect_err("an i64 cannot hold it");

    assert!(matches!(
        error,
        ConfigError::Environment {
            source: EnvError::Number { name },
        } if name == INTERVAL_VARIABLE
    ));
}

#[rstest]
#[case::zero("0")]
#[case::negative("-3")]
fn a_non_positive_interval_is_refused(#[case] text: &str) {
    let error = load("model_list: []\n", &[(INTERVAL_VARIABLE, text)])
        .expect_err("the interval is refused");

    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == INTERVAL_VARIABLE
    ));
}
