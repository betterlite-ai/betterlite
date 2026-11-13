use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::gcs_mock::settings::{
    DEFAULT_LATENCY, ENABLE_ENV, GcsMockConfigError, GcsMockSettings, LATENCY_ENV,
    should_use_gcs_mock,
};

use super::support::environment;

#[rstest]
#[case::true_word("true")]
#[case::one("1")]
#[case::yes("yes")]
#[case::on("on")]
#[case::upper_case("TRUE")]
fn truthy_flags_enable_the_mock(#[case] value: &str) {
    let enabled = should_use_gcs_mock(&environment(&[(ENABLE_ENV, value)])).unwrap();
    assert!(enabled);
}

#[rstest]
#[case::false_word("false")]
#[case::zero("0")]
#[case::no("no")]
#[case::off("off")]
fn falsy_flags_disable_the_mock(#[case] value: &str) {
    let enabled = should_use_gcs_mock(&environment(&[(ENABLE_ENV, value)])).unwrap();
    assert!(!enabled);
}

#[test]
fn absent_flag_disables_the_mock() {
    assert!(!should_use_gcs_mock(&BTreeMap::new()).unwrap());
}

#[test]
fn empty_flag_disables_the_mock() {
    assert!(!should_use_gcs_mock(&environment(&[(ENABLE_ENV, "")])).unwrap());
}

#[rstest]
#[case::unknown_word("maybe")]
#[case::out_of_range_number("2")]
fn unknown_flags_name_the_variable(#[case] value: &str) {
    let outcome = should_use_gcs_mock(&environment(&[(ENABLE_ENV, value)]));
    assert!(matches!(
        outcome,
        Err(GcsMockConfigError::Environment { .. })
    ));
}

#[test]
fn latency_defaults_to_150_milliseconds() {
    let settings =
        GcsMockSettings::from_environment(&environment(&[(ENABLE_ENV, "true")])).unwrap();
    assert_eq!(settings.latency, DEFAULT_LATENCY);
    assert_eq!(settings.latency, Duration::from_millis(150));
}

#[test]
fn latency_reads_milliseconds_from_the_environment() {
    let settings = GcsMockSettings::from_environment(&environment(&[
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, "25"),
    ]))
    .unwrap();
    assert_eq!(settings.latency, Duration::from_millis(25));
}

#[test]
fn latency_accepts_fractional_milliseconds() {
    let settings = GcsMockSettings::from_environment(&environment(&[
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, "150.5"),
    ]))
    .unwrap();
    assert_eq!(settings.latency, Duration::from_micros(150_500));
}

#[test]
fn empty_latency_uses_the_default() {
    let settings =
        GcsMockSettings::from_environment(&environment(&[(ENABLE_ENV, "true"), (LATENCY_ENV, "")]))
            .unwrap();
    assert_eq!(settings.latency, DEFAULT_LATENCY);
}

#[rstest]
#[case::negative("-1")]
#[case::huge("1e300")]
fn out_of_range_latency_is_rejected(#[case] value: &str) {
    let outcome = GcsMockSettings::from_environment(&environment(&[
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, value),
    ]));
    assert!(matches!(outcome, Err(GcsMockConfigError::Latency { .. })));
}

#[rstest]
#[case::word("slow")]
#[case::comma("1,5")]
fn malformed_latency_names_the_variable(#[case] value: &str) {
    let outcome = GcsMockSettings::from_environment(&environment(&[
        (ENABLE_ENV, "true"),
        (LATENCY_ENV, value),
    ]));
    assert!(matches!(
        outcome,
        Err(GcsMockConfigError::Environment { .. })
    ));
}
