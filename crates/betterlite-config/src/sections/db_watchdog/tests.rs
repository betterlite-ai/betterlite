#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    ENABLED_VARIABLE, INTERVAL_VARIABLE, PROBE_TIMEOUT_VARIABLE, RECONNECT_TIMEOUT_VARIABLE,
    SCHEMA_VARIABLE, Settings,
};

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.db_watchdog)
}

#[test]
fn watchdog_defaults() {
    let settings = settings(&[]).expect("the defaults load");
    assert_eq!(
        settings,
        Settings {
            enabled: true,
            interval: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(5),
            reconnect_timeout: Duration::from_secs(30),
            database_schema: "public".to_owned(),
        }
    );
}

#[test]
fn watchdog_floors() {
    let settings = settings(&[
        (INTERVAL_VARIABLE, "0"),
        (PROBE_TIMEOUT_VARIABLE, "0.1"),
        (RECONNECT_TIMEOUT_VARIABLE, "0.5"),
    ])
    .expect("values below the floor clamp");
    assert_eq!(settings.interval, Duration::from_secs(5));
    assert_eq!(settings.probe_timeout, Duration::from_millis(500));
    assert_eq!(settings.reconnect_timeout, Duration::from_secs(1));
}

#[test]
fn watchdog_negative_interval_takes_the_floor() {
    let settings = settings(&[(INTERVAL_VARIABLE, "-4")]).expect("the floor applies below zero");
    assert_eq!(settings.interval, Duration::from_secs(5));
}

#[test]
fn watchdog_reads_values_above_the_floor() {
    let settings = settings(&[
        (ENABLED_VARIABLE, "false"),
        (INTERVAL_VARIABLE, "45"),
        (PROBE_TIMEOUT_VARIABLE, "2.5"),
        (RECONNECT_TIMEOUT_VARIABLE, "90"),
        (SCHEMA_VARIABLE, "litellm"),
    ])
    .expect("values above the floor load");
    assert_eq!(
        settings,
        Settings {
            enabled: false,
            interval: Duration::from_secs(45),
            probe_timeout: Duration::from_millis(2500),
            reconnect_timeout: Duration::from_secs(90),
            database_schema: "litellm".to_owned(),
        }
    );
}

#[rstest]
#[case::true_word("true", true)]
#[case::upper_case(" TRUE ", true)]
#[case::one("1", true)]
#[case::yes("yes", true)]
#[case::on("on", true)]
#[case::false_word("false", false)]
#[case::zero("0", false)]
#[case::no("no", false)]
#[case::off("off", false)]
fn watchdog_enabled_forms(#[case] value: &str, #[case] enabled: bool) {
    let settings = settings(&[(ENABLED_VARIABLE, value)]).expect("the flag loads");
    assert_eq!(settings.enabled, enabled);
}

#[test]
fn watchdog_empty_values_take_the_defaults() {
    let settings = settings(&[
        (ENABLED_VARIABLE, ""),
        (INTERVAL_VARIABLE, "  "),
        (PROBE_TIMEOUT_VARIABLE, ""),
        (RECONNECT_TIMEOUT_VARIABLE, ""),
        (SCHEMA_VARIABLE, ""),
    ])
    .expect("empty values count as unset");
    assert_eq!(settings, Settings::default());
}

#[rstest]
#[case::flag_word(ENABLED_VARIABLE, "sometimes")]
#[case::interval_word(INTERVAL_VARIABLE, "soon")]
#[case::interval_fraction(INTERVAL_VARIABLE, "1.5")]
#[case::interval_too_large(INTERVAL_VARIABLE, "99999999999999999999")]
#[case::probe_word(PROBE_TIMEOUT_VARIABLE, "later")]
#[case::reconnect_word(RECONNECT_TIMEOUT_VARIABLE, "never")]
fn watchdog_invalid_startup(#[case] name: &str, #[case] value: &str) {
    let error = settings(&[(name, value)]).expect_err("malformed values stop startup");
    assert!(error.to_string().contains(name), "{error}");
}

#[rstest]
#[case::probe_infinity(PROBE_TIMEOUT_VARIABLE)]
#[case::reconnect_infinity(RECONNECT_TIMEOUT_VARIABLE)]
fn watchdog_nonfinite_values(#[case] name: &str) {
    let settings = settings(&[(name, "1e999")]).expect("an overflowing value stays unbounded");
    let actual = if name == PROBE_TIMEOUT_VARIABLE {
        settings.probe_timeout
    } else {
        settings.reconnect_timeout
    };
    assert_eq!(actual, Duration::MAX);
}
