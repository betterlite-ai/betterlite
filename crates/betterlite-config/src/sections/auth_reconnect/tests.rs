#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{
    AUTH_LOCK_VARIABLE, AUTH_TIMEOUT_VARIABLE, COOLDOWN_VARIABLE, ESCALATION_VARIABLE, Settings,
    WATCHDOG_VARIABLE,
};

fn settings(variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []\n")
        .map(|config| config.sections.auth_reconnect)
}

#[test]
fn reconnect_defaults() {
    let settings = settings(&[]).expect("the defaults load");
    assert_eq!(settings.cooldown, Duration::from_secs(15));
    assert_eq!(settings.escalation_threshold, 3);
    assert_eq!(settings.watchdog_timeout, Duration::from_secs(30));
    assert_eq!(settings.auth_timeout, Duration::from_secs(2));
    assert_eq!(settings.auth_lock_timeout, Duration::from_millis(100));
}

#[test]
fn reconnect_lower_bounds() {
    let settings = settings(&[
        (COOLDOWN_VARIABLE, "0"),
        (ESCALATION_VARIABLE, "-2"),
        (WATCHDOG_VARIABLE, "0.5"),
        (AUTH_TIMEOUT_VARIABLE, "0.1"),
        (AUTH_LOCK_VARIABLE, "-1"),
    ])
    .expect("values below the minimum clamp");
    assert_eq!(settings.cooldown, Duration::from_secs(1));
    assert_eq!(settings.escalation_threshold, 1);
    assert_eq!(settings.watchdog_timeout, Duration::from_secs(1));
    assert_eq!(settings.auth_timeout, Duration::from_millis(500));
    assert_eq!(settings.auth_lock_timeout, Duration::ZERO);
}

#[rstest]
#[case::cooldown_word(COOLDOWN_VARIABLE, "soon")]
#[case::cooldown_fraction(COOLDOWN_VARIABLE, "1.5")]
#[case::cooldown_empty(COOLDOWN_VARIABLE, "")]
#[case::escalation_empty(ESCALATION_VARIABLE, "  ")]
#[case::escalation_word(ESCALATION_VARIABLE, "many")]
#[case::watchdog_word(WATCHDOG_VARIABLE, "later")]
#[case::watchdog_empty(WATCHDOG_VARIABLE, "")]
#[case::auth_word(AUTH_TIMEOUT_VARIABLE, "two")]
#[case::auth_lock_empty(AUTH_LOCK_VARIABLE, "")]
fn reconnect_invalid_startup(#[case] name: &str, #[case] value: &str) {
    let error = settings(&[(name, value)]).expect_err("malformed values stop startup");
    assert!(error.to_string().contains(name), "{error}");
}

#[rstest]
#[case::watchdog_infinity(WATCHDOG_VARIABLE, "infinity", 0, Duration::MAX)]
#[case::watchdog_nan(WATCHDOG_VARIABLE, "NaN", 0, Duration::from_secs(1))]
#[case::auth_infinity(AUTH_TIMEOUT_VARIABLE, "+inf", 1, Duration::MAX)]
#[case::auth_nan(AUTH_TIMEOUT_VARIABLE, "nan", 1, Duration::from_millis(500))]
#[case::auth_negative_infinity(AUTH_TIMEOUT_VARIABLE, "-inf", 1, Duration::from_millis(500))]
#[case::lock_infinity(AUTH_LOCK_VARIABLE, "Inf", 2, Duration::MAX)]
#[case::lock_nan(AUTH_LOCK_VARIABLE, "nan", 2, Duration::ZERO)]
fn reconnect_nonfinite_values(
    #[case] name: &str,
    #[case] value: &str,
    #[case] selector: usize,
    #[case] expected: Duration,
) {
    let settings =
        settings(&[(name, value)]).expect("nonfinite values keep the max(lower, value) semantics");
    let actual = match selector {
        0 => settings.watchdog_timeout,
        1 => settings.auth_timeout,
        _ => settings.auth_lock_timeout,
    };
    assert_eq!(actual, expected);
}
