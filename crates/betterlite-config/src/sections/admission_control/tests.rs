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
#[case::list_in_flight("general_settings:\n  max_in_flight_requests_per_worker: [1]", None)]
#[case::map_in_flight(
    "general_settings:\n  max_in_flight_requests_per_worker: {limit: 1}",
    None
)]
#[case::list_queue(
    "general_settings:\n  max_in_flight_requests_per_worker: 2\n  max_queued_requests_per_worker: [1]",
    None
)]
#[case::map_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 2\n  admission_queue_timeout_seconds: {seconds: 1}",
    None
)]
#[case::null_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 2\n  admission_queue_timeout_seconds: null",
    None
)]
#[case::plain_exponent(
    "general_settings:\n  max_in_flight_requests_per_worker: 1e3",
    Some((1000, 1000, 1.0_f64))
)]
#[case::plain_exponent_fraction(
    "general_settings:\n  max_in_flight_requests_per_worker: 1.0e3",
    Some((1000, 1000, 1.0_f64))
)]
#[case::exponent_text(
    "general_settings:\n  max_in_flight_requests_per_worker: \"1e3\"",
    None
)]
#[case::trailing_dot_text("general_settings:\n  max_in_flight_requests_per_worker: \"4.\"", None)]
#[case::underscored_text(
    "general_settings:\n  max_in_flight_requests_per_worker: \"1_000\"",
    Some((1000, 1000, 1.0_f64))
)]
#[case::underscored_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: \"1_000\"",
    Some((1, 1, 1000.0_f64))
)]
#[case::edge_underscore_text(
    "general_settings:\n  max_in_flight_requests_per_worker: \"_1\"",
    None
)]
#[case::edge_underscore_timeout(
    "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: \"1_\"",
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

const IN_FLIGHT: &str = "general_settings:\n  max_in_flight_requests_per_worker: os.environ/V";
const QUEUE: &str = "general_settings:\n  max_in_flight_requests_per_worker: 2\n  max_queued_requests_per_worker: os.environ/V";
const TIMEOUT: &str = "general_settings:\n  max_in_flight_requests_per_worker: 2\n  admission_queue_timeout_seconds: os.environ/V";

#[rstest]
#[case::exponent(IN_FLIGHT, Some("1e3"), None)]
#[case::trailing_dot(IN_FLIGHT, Some("4."), None)]
#[case::underscored(IN_FLIGHT, Some("1_000"), Some((1000, 1000, 1.0_f64)))]
#[case::empty_in_flight(IN_FLIGHT, Some(""), None)]
#[case::empty_queue(QUEUE, Some(""), None)]
#[case::unset_queue_uses_in_flight(QUEUE, None, Some((2, 2, 1.0_f64)))]
#[case::unset_timeout(TIMEOUT, None, None)]
#[case::empty_timeout(TIMEOUT, Some(""), None)]
#[case::underscored_timeout(TIMEOUT, Some("1_000"), Some((2, 2, 1000.0_f64)))]
#[case::infinite_timeout(TIMEOUT, Some("inf"), Some((2, 2, Duration::MAX.as_secs_f64())))]
fn environment_values_use_the_text_grammar(
    #[case] source: &str,
    #[case] value: Option<&str>,
    #[case] expected: Option<(usize, usize, f64)>,
) {
    let variables: Vec<(&str, &str)> = value.map(|text| ("V", text)).into_iter().collect();
    assert_admission(source, &variables, expected);
}

#[test]
fn unset_environment_reference_disables_admission() {
    let lookup = BTreeMap::<String, String>::new();
    let settings = ConfigLoader { lookup }
        .load("general_settings:\n  max_in_flight_requests_per_worker: os.environ/IN_FLIGHT")
        .unwrap()
        .sections
        .admission_control;
    assert_eq!(settings, Settings::default());
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
fn a_timeout_past_the_duration_range_never_expires() {
    let settings = settings(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  admission_queue_timeout_seconds: 1e30",
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
