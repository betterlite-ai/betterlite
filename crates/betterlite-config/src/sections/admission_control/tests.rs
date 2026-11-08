#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::admission_control::{Limits, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .admission_control
}

#[rstest]
#[case::absent("general_settings: {}", None)]
#[case::explicit_null("general_settings:\n  max_in_flight_requests_per_worker: null", None)]
#[case::zero("general_settings:\n  max_in_flight_requests_per_worker: 0", None)]
#[case::negative("general_settings:\n  max_in_flight_requests_per_worker: -2", None)]
#[case::fractional("general_settings:\n  max_in_flight_requests_per_worker: 2.5", None)]
#[case::not_a_number("general_settings:\n  max_in_flight_requests_per_worker: many", None)]
#[case::boolean("general_settings:\n  max_in_flight_requests_per_worker: true", None)]
#[case::zero_queue(
    "general_settings:\n  max_in_flight_requests_per_worker: 3\n  max_queued_requests_per_worker: 0",
    Some((3, 0, 1.0_f64))
)]
#[case::absent_queue_equals_in_flight(
    "general_settings:\n  max_in_flight_requests_per_worker: 3",
    Some((3, 3, 1.0_f64))
)]
#[case::explicit_values(
    "general_settings:\n  max_in_flight_requests_per_worker: 3\n  max_queued_requests_per_worker: 5\n  admission_queue_timeout_seconds: 0.25",
    Some((3, 5, 0.25_f64))
)]
#[case::integral_float(
    "general_settings:\n  max_in_flight_requests_per_worker: 4.0",
    Some((4, 4, 1.0_f64))
)]
#[case::numeric_text(
    "general_settings:\n  max_in_flight_requests_per_worker: \"4.0\"\n  admission_queue_timeout_seconds: \"0.5\"",
    Some((4, 4, 0.5_f64))
)]
#[case::negative_queue(
    "general_settings:\n  max_in_flight_requests_per_worker: 3\n  max_queued_requests_per_worker: -1",
    None
)]
#[case::zero_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 3\n  admission_queue_timeout_seconds: 0",
    None
)]
#[case::null_queue_uses_in_flight(
    "general_settings:\n  max_in_flight_requests_per_worker: 2\n  max_queued_requests_per_worker: null",
    Some((2, 2, 1.0_f64))
)]
#[case::nan_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 2\n  admission_queue_timeout_seconds: nan",
    None
)]
fn converts_admission_settings(
    #[case] source: &str,
    #[case] expected: Option<(usize, usize, f64)>,
) {
    assert_admission(source, &[], expected);
}

fn assert_admission(
    source: &str,
    variables: &[(&str, &str)],
    expected: Option<(usize, usize, f64)>,
) {
    let settings = settings(source, variables);
    match expected {
        None => assert_eq!(settings, Settings::default()),
        Some((max_in_flight, max_queued, timeout)) => {
            let limits: Limits = settings.limits.unwrap();
            assert_eq!(limits.max_in_flight_requests, max_in_flight);
            assert_eq!(limits.max_queued_requests, max_queued);
            assert_eq!(
                limits.queue_timeout.as_secs_f64().to_bits(),
                timeout.to_bits()
            );
        }
    }
}

#[rstest]
#[case::environment_in_flight(
    "general_settings:\n  max_in_flight_requests_per_worker: os.environ/IN_FLIGHT",
    &[("IN_FLIGHT", "2")],
    Some((2, 2, 1.0_f64))
)]
#[case::empty_environment_is_unset(
    "general_settings:\n  max_in_flight_requests_per_worker: os.environ/IN_FLIGHT",
    &[("IN_FLIGHT", "")],
    None
)]
#[case::invalid_environment(
    "general_settings:\n  max_in_flight_requests_per_worker: os.environ/IN_FLIGHT",
    &[("IN_FLIGHT", "many")],
    None
)]
#[case::environment_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: os.environ/TIMEOUT",
    &[("TIMEOUT", "2.5")],
    Some((1, 1, 2.5_f64))
)]
fn resolves_environment_references(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<(usize, usize, f64)>,
) {
    assert_admission(source, variables, expected);
}

#[test]
fn missing_environment_reference_fails_the_load() {
    let lookup = BTreeMap::<String, String>::new();
    let error = ConfigLoader { lookup }
        .load("general_settings:\n  max_in_flight_requests_per_worker: os.environ/IN_FLIGHT")
        .unwrap_err();
    assert!(error.to_string().contains("IN_FLIGHT"), "{error}");
}

#[test]
fn one_invalid_setting_disables_every_limit() {
    let settings = settings(
        "general_settings:\n  max_in_flight_requests_per_worker: 3\n  admission_queue_timeout_seconds: -1",
        &[],
    );
    assert_eq!(settings, Settings::default());
}

#[test]
fn infinite_timeout_never_expires() {
    let settings = settings(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: inf",
        &[],
    );
    assert_eq!(settings.limits.unwrap().queue_timeout, Duration::MAX);
}

#[test]
fn sub_nanosecond_timeout_rounds_to_the_nearest_nanosecond() {
    let settings = settings(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: 0.00000042",
        &[],
    );
    assert_eq!(
        settings.limits.unwrap().queue_timeout,
        Duration::from_nanos(420)
    );
}
