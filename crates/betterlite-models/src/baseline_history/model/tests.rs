#![cfg(test)]

use rstest::rstest;

use super::{BaselineHistory, BaselineObservation, Outcome, Raw};
use crate::baseline_history::error::ObservationError;

fn observation_json_with(field: &str, value: &str) -> String {
    let mut fields = vec![
        ("request_id", "\"req-1\""),
        ("started_at", "0.0"),
        ("available_at", "0.0"),
        ("outcome", "\"complete\""),
        ("baseline_equivalent", "true"),
    ];
    match fields.iter_mut().find(|(name, _)| *name == field) {
        Some(entry) => entry.1 = value,
        None if field.is_empty() => {}
        None => fields.push((field, value)),
    }
    let body = fields
        .into_iter()
        .map(|(name, rendered)| format!("\"{name}\": {rendered}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{body}}}")
}

fn minimal_observation_json() -> String {
    observation_json_with("", "")
}

#[test]
fn a_minimal_observation_fills_in_every_default() {
    let observation: BaselineObservation =
        serde_json::from_str(&minimal_observation_json()).expect("minimal observation");
    assert_eq!(observation.minimum_cache_tokens, 0);
    assert_eq!(observation.reason, None);
    assert_eq!(observation.usage, None);
    assert_eq!(observation.plan, None);
    assert_eq!(observation.outcome, Outcome::Complete);
}

#[rstest]
#[case::wrong_version("version", "4")]
#[case::empty_request_id("request_id", "\"\"")]
#[case::negative_started_at("started_at", "-1.0")]
#[case::overflowing_available_at_literal("available_at", "1e400")]
#[case::negative_minimum_cache_tokens("minimum_cache_tokens", "-1")]
fn an_invalid_field_is_rejected_at_the_deserialize_boundary(
    #[case] field: &str,
    #[case] value: &str,
) {
    let json = observation_json_with(field, value);
    let result: Result<BaselineObservation, _> = serde_json::from_str(&json);
    result.unwrap_err();
}

#[rstest]
#[case::started_at_infinity(f64::INFINITY, 0.0, ObservationError::InvalidStartedAt)]
#[case::started_at_nan(f64::NAN, 0.0, ObservationError::InvalidStartedAt)]
#[case::available_at_infinity(0.0, f64::INFINITY, ObservationError::InvalidAvailableAt)]
#[case::available_at_nan(0.0, f64::NAN, ObservationError::InvalidAvailableAt)]
fn a_non_finite_timestamp_is_rejected_by_its_own_check(
    #[case] started_at: f64,
    #[case] available_at: f64,
    #[case] expected: ObservationError,
) {
    let raw = Raw {
        version: 3,
        request_id: "req-1".to_owned(),
        started_at,
        available_at,
        outcome: Outcome::Complete,
        baseline_equivalent: true,
        usage: None,
        plan: None,
        minimum_cache_tokens: 0,
        reason: None,
    };
    assert_eq!(BaselineObservation::try_from(raw).unwrap_err(), expected);
}

#[test]
fn an_unknown_field_is_rejected() {
    let json = observation_json_with("unexpected", "true");
    let result: Result<BaselineObservation, _> = serde_json::from_str(&json);
    result.unwrap_err();
}

#[test]
fn outcome_round_trips_through_its_snake_case_wire_form() {
    let wire = serde_json::to_string(&Outcome::ResponseCache).expect("serialize outcome");
    assert_eq!(wire, "\"response_cache\"");
    let back: Outcome = serde_json::from_str(&wire).expect("deserialize outcome");
    assert_eq!(back, Outcome::ResponseCache);
}

#[test]
fn a_default_history_starts_equivalent() {
    assert!(BaselineHistory::default().equivalent);
}
