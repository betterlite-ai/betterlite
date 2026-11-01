#![cfg(test)]

use rstest::rstest;

use super::{BaselineHistory, BaselineObservation, Outcome};

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
#[case::infinite_available_at("available_at", "1e400")]
#[case::negative_minimum_cache_tokens("minimum_cache_tokens", "-1")]
fn an_invalid_field_is_rejected_at_the_deserialize_boundary(
    #[case] field: &str,
    #[case] value: &str,
) {
    let json = observation_json_with(field, value);
    let result: Result<BaselineObservation, _> = serde_json::from_str(&json);
    result.unwrap_err();
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
