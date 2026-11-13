#![cfg(test)]

use std::collections::BTreeMap;
use std::num::ParseFloatError;
use std::time::{Duration, TryFromFloatSecsError};

use rstest::rstest;

use super::{
    ENABLED_VARIABLE, INITIAL_INTERVAL_VARIABLE, MAX_INTERVAL_VARIABLE, PollSettings,
    PollSettingsError, TIMEOUT_VARIABLE,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn defaults_when_every_variable_is_absent() {
    let settings = PollSettings::from_env_map(&BTreeMap::new()).unwrap();
    assert_eq!(
        settings,
        PollSettings {
            initial_interval: Duration::from_secs(5),
            max_interval: Duration::from_secs(60),
            timeout: Duration::from_secs(3600),
            enabled: true,
        }
    );
}

#[test]
fn reads_each_override() {
    let settings = PollSettings::from_env_map(&environment(&[
        (INITIAL_INTERVAL_VARIABLE, "2.5"),
        (MAX_INTERVAL_VARIABLE, "30"),
        (TIMEOUT_VARIABLE, "120"),
        (ENABLED_VARIABLE, "false"),
    ]))
    .unwrap();
    assert_eq!(
        settings,
        PollSettings {
            initial_interval: Duration::from_millis(2500),
            max_interval: Duration::from_secs(30),
            timeout: Duration::from_secs(120),
            enabled: false,
        }
    );
}

#[test]
fn a_number_may_carry_surrounding_whitespace_as_python_float_accepts_it() {
    let settings = PollSettings::from_env_map(&environment(&[(TIMEOUT_VARIABLE, " 7 ")]))
        .expect("float() strips surrounding whitespace");
    assert_eq!(settings.timeout, Duration::from_secs(7));
}

#[rstest]
#[case::exact("true", true)]
#[case::uppercase("TRUE", true)]
#[case::mixed_case("True", true)]
#[case::leading_space(" true", false)]
#[case::trailing_space("true ", false)]
#[case::one("1", false)]
#[case::yes("yes", false)]
#[case::on("on", false)]
#[case::empty("", false)]
fn enables_polling_only_for_the_lowercased_text_true(#[case] text: &str, #[case] expected: bool) {
    let settings = PollSettings::from_env_map(&environment(&[(ENABLED_VARIABLE, text)])).unwrap();
    assert_eq!(settings.enabled, expected);
}

#[rstest]
#[case::initial(INITIAL_INTERVAL_VARIABLE, "fast")]
#[case::maximum(MAX_INTERVAL_VARIABLE, "")]
#[case::maximum_blank(MAX_INTERVAL_VARIABLE, "  ")]
#[case::timeout(TIMEOUT_VARIABLE, "1_0")]
#[case::timeout_suffix(TIMEOUT_VARIABLE, "60s")]
fn rejects_an_unparsable_number_naming_the_variable(
    #[case] variable: &'static str,
    #[case] value: &str,
) {
    let error = PollSettings::from_env_map(&environment(&[(variable, value)]))
        .expect_err("an unparsable value is a startup error");
    assert_eq!(
        error,
        PollSettingsError::NotANumber {
            name: variable,
            value: value.to_owned(),
            source: unparsable(value),
        }
    );
    assert!(error.to_string().contains(variable));
}

fn unparsable(value: &str) -> ParseFloatError {
    value
        .trim()
        .parse::<f64>()
        .expect_err("the fixture text is not a number")
}

fn out_of_range(value: &str) -> TryFromFloatSecsError {
    let seconds: f64 = value.trim().parse().expect("the fixture text is a float");
    Duration::try_from_secs_f64(seconds).expect_err("the fixture value is out of range")
}

#[rstest]
#[case::negative("-5")]
#[case::negative_infinity("-inf")]
#[case::nan("nan")]
fn rejects_a_number_a_duration_cannot_hold_naming_the_variable(#[case] value: &str) {
    let error = PollSettings::from_env_map(&environment(&[(TIMEOUT_VARIABLE, value)]))
        .expect_err("a duration cannot hold a negative or nan number of seconds");
    assert_eq!(
        error,
        PollSettingsError::OutOfRange {
            name: TIMEOUT_VARIABLE,
            value: value.to_owned(),
            source: out_of_range(value),
        }
    );
    assert!(error.to_string().contains(TIMEOUT_VARIABLE));
}

#[test]
fn clamps_a_timeout_past_durations_range_to_its_maximum() {
    let settings = PollSettings::from_env_map(&environment(&[(TIMEOUT_VARIABLE, "inf")]))
        .expect("LiteLLM's float accepts infinity and polls without a timeout");
    assert_eq!(settings.timeout, Duration::MAX);
}

#[rstest]
#[case::infinity("inf")]
#[case::finite_overflow("1e30")]
fn clamps_a_max_interval_past_durations_range_to_its_maximum(#[case] value: &str) {
    let settings = PollSettings::from_env_map(&environment(&[(MAX_INTERVAL_VARIABLE, value)]))
        .expect("LiteLLM's float accepts values a Duration cannot hold and polls without a cap");
    assert_eq!(settings.max_interval, Duration::MAX);
}
