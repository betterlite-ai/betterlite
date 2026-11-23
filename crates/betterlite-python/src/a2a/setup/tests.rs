#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_a2a::client::{A2A_TIMEOUT_ENV, DEFAULT_A2A_AGENT_TIMEOUT, resolve_timeout};
use rstest::rstest;
use secrecy::SecretString;

use super::{card_headers, runtime_config, trace_ids};
use crate::a2a::error::A2aCallError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

#[rstest]
#[case::absent(&[], DEFAULT_A2A_AGENT_TIMEOUT)]
#[case::empty(&[(A2A_TIMEOUT_ENV, "")], DEFAULT_A2A_AGENT_TIMEOUT)]
#[case::seconds(&[(A2A_TIMEOUT_ENV, "3")], Duration::from_secs(3))]
#[case::fractional(&[(A2A_TIMEOUT_ENV, "0.5")], Duration::from_millis(500))]
fn resolve_timeout_defaults_from_the_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Duration,
) {
    assert_eq!(
        resolve_timeout(&environment(pairs), None).unwrap(),
        expected
    );
}

#[rstest]
#[case::not_a_number(&[(A2A_TIMEOUT_ENV, "soon")], "soon")]
#[case::negative(&[(A2A_TIMEOUT_ENV, "-2")], "-2")]
fn resolve_timeout_rejects_a_bad_environment_value(
    #[case] pairs: &[(&str, &str)],
    #[case] raw: &str,
) {
    let payload = A2aCallError::from(
        resolve_timeout(&environment(pairs), None).expect_err("a bad timeout is rejected"),
    )
    .payload();
    assert_eq!(payload.kind, "A2AError");
    assert!(payload.message.contains(raw));
    assert!(payload.message.contains(A2A_TIMEOUT_ENV));
}

#[test]
fn card_headers_keep_every_given_pair() {
    let extra = secrets(&[("X-One", "1"), ("x-two", "2")]);
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
    let extra = secrets(&[(name, value)]);
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
