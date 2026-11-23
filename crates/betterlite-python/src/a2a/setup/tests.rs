#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_a2a::client::{A2A_TIMEOUT_ENV, DEFAULT_A2A_AGENT_TIMEOUT};
use betterlite_a2a::errors::A2AError;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{card_headers, resolve_timeout, runtime_config, secret_headers, trace_ids};
use crate::a2a::error::A2aCallError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::fractional(Some(12.5), Ok(Duration::from_millis(12_500)))]
#[case::zero(Some(0.0), Ok(Duration::ZERO))]
#[case::negative(Some(-1.0), Err("timeout"))]
#[case::not_a_number(Some(f64::NAN), Err("timeout"))]
#[case::infinite(Some(f64::INFINITY), Err("timeout"))]
fn resolve_timeout_uses_the_configured_seconds(
    #[case] configured: Option<f64>,
    #[case] expected: Result<Duration, &str>,
) {
    match (resolve_timeout(&environment(&[]), configured), expected) {
        (Ok(found), Ok(want)) => assert_eq!(found, want),
        (Err(A2aCallError::Timeout { field }), Err(want)) => assert_eq!(field, want),
        (found, want) => panic!("expected {want:?}, got {found:?}"),
    }
}

#[rstest]
#[case::absent(&[], Ok(DEFAULT_A2A_AGENT_TIMEOUT))]
#[case::empty(&[(A2A_TIMEOUT_ENV, "")], Ok(DEFAULT_A2A_AGENT_TIMEOUT))]
#[case::seconds(&[(A2A_TIMEOUT_ENV, "3")], Ok(Duration::from_secs(3)))]
#[case::fractional(&[(A2A_TIMEOUT_ENV, "0.5")], Ok(Duration::from_millis(500)))]
#[case::not_a_number(&[(A2A_TIMEOUT_ENV, "soon")], Err("soon"))]
#[case::negative(&[(A2A_TIMEOUT_ENV, "-2")], Err("-2"))]
fn resolve_timeout_reads_the_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Result<Duration, &str>,
) {
    match (resolve_timeout(&environment(pairs), None), expected) {
        (Ok(found), Ok(want)) => assert_eq!(found, want),
        (Err(A2aCallError::A2A(A2AError::Base { message, .. })), Err(want)) => {
            assert!(message.contains(want));
            assert!(message.contains(A2A_TIMEOUT_ENV));
        }
        (found, want) => panic!("expected {want:?}, got {found:?}"),
    }
}

#[test]
fn secret_headers_keep_every_given_pair() {
    let extra = environment(&[("X-One", "1"), ("X-Two", "2")]);
    let headers = secret_headers(Some(&extra));
    assert_eq!(headers.len(), 2);
    assert_eq!(
        headers.get("X-Two").map(|value| value.expose_secret()),
        Some("2")
    );
    assert!(secret_headers(None).is_empty());
}

#[test]
fn card_headers_keep_every_given_pair() {
    let extra = environment(&[("X-One", "1"), ("x-two", "2")]);
    let headers = match card_headers(Some(&extra)) {
        Ok(headers) => headers,
        Err(error) => panic!("valid headers were rejected: {error}"),
    };
    assert_eq!(headers.len(), 2);
    assert_eq!(
        headers
            .get("x-one")
            .map(|value| value.to_str().unwrap_or("")),
        Some("1")
    );
}

#[rstest]
#[case::bad_name("bad name", "1", "name")]
#[case::bad_value("X-One", "one\ntwo", "value")]
fn card_headers_reject_an_invalid_pair(
    #[case] name: &str,
    #[case] value: &str,
    #[case] field: &str,
) {
    let extra = environment(&[(name, value)]);
    match (card_headers(Some(&extra)), field) {
        (Err(A2aCallError::HeaderName { .. }), "name") => {}
        (Err(A2aCallError::HeaderValue { .. }), "value") => {}
        (found, want) => panic!("expected the {want} to be rejected, got {found:?}"),
    }
}

#[test]
fn trace_ids_repeat_the_callers_trace_id() {
    let ids = trace_ids(Some(String::from("trace-1")));
    assert_eq!(ids(), "trace-1");
    assert_eq!(ids(), "trace-1");
}

#[test]
fn trace_ids_make_a_new_id_per_call_without_one() {
    let ids = trace_ids(None);
    let first = ids();
    let second = ids();
    assert!(!first.is_empty());
    assert_ne!(first, second);
}

#[test]
fn runtime_config_loads_the_empty_document() {
    runtime_config(&environment(&[])).unwrap();
}
