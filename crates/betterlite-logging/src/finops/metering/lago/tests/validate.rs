#![cfg(test)]

use rstest::rstest;

use crate::finops::metering::lago::settings::{LagoSettings, LagoSettingsError};
use crate::finops::metering::support::variables;

#[rstest]
#[case::api_key_missing(
    &[("LAGO_API_BASE", "https://lago.test"), ("LAGO_API_EVENT_CODE", "code")],
    vec!["LAGO_API_KEY"]
)]
#[case::api_base_missing(
    &[("LAGO_API_KEY", "key"), ("LAGO_API_EVENT_CODE", "code")],
    vec!["LAGO_API_BASE"]
)]
#[case::event_code_missing(
    &[("LAGO_API_KEY", "key"), ("LAGO_API_BASE", "https://lago.test")],
    vec!["LAGO_API_EVENT_CODE"]
)]
#[case::all_missing(
    &[],
    vec!["LAGO_API_KEY", "LAGO_API_BASE", "LAGO_API_EVENT_CODE"]
)]
fn validate_reports_the_missing_keys(
    #[case] pairs: &[(&'static str, &'static str)],
    #[case] expected: Vec<&'static str>,
) {
    let error = LagoSettings::from_environment(&variables(pairs)).unwrap_err();
    let LagoSettingsError::MissingKeys { missing_keys } = error else {
        panic!("the settings error is not MissingKeys");
    };
    assert_eq!(missing_keys, expected);
}

#[rstest]
#[case::api_key("LAGO_API_KEY")]
#[case::api_base("LAGO_API_BASE")]
#[case::event_code("LAGO_API_EVENT_CODE")]
fn validate_counts_an_empty_value_as_unset(#[case] empty: &'static str) {
    let mut values = variables(&[
        ("LAGO_API_KEY", "key"),
        ("LAGO_API_BASE", "https://lago.test"),
        ("LAGO_API_EVENT_CODE", "code"),
    ]);
    values.insert(empty.to_owned(), String::new());
    let error = LagoSettings::from_environment(&values).unwrap_err();
    let LagoSettingsError::MissingKeys { missing_keys } = error else {
        panic!("the settings error is not MissingKeys");
    };
    assert_eq!(missing_keys, vec![empty]);
}

#[test]
fn validate_accepts_a_complete_environment() {
    let values = variables(&[
        ("LAGO_API_KEY", "key"),
        ("LAGO_API_BASE", "https://lago.test"),
        ("LAGO_API_EVENT_CODE", "code"),
    ]);
    LagoSettings::from_environment(&values).unwrap();
}
