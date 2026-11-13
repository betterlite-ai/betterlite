#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

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
        }
    );
    assert!(error.to_string().contains(variable));
}

#[rstest]
#[case::negative("-5")]
#[case::infinity("inf")]
#[case::nan("nan")]
fn rejects_a_number_a_duration_cannot_hold_naming_the_variable(#[case] value: &str) {
    let error = PollSettings::from_env_map(&environment(&[(TIMEOUT_VARIABLE, value)]))
        .expect_err("a duration cannot hold a negative or non-finite number of seconds");
    assert_eq!(
        error,
        PollSettingsError::OutOfRange {
            name: TIMEOUT_VARIABLE,
            value: value.to_owned(),
        }
    );
    assert!(error.to_string().contains(TIMEOUT_VARIABLE));
}
