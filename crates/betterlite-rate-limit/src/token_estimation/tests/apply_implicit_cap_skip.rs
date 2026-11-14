#![cfg(test)]

use rstest::rstest;

use super::output_cap::{CapEndpoint, apply_implicit_output_cap};
use super::support::request;

const UNCAPPED: &str = r#"{"messages":[]}"#;

#[rstest]
#[case::no_configured_limit(None)]
#[case::at_the_baseline(Some(4096))]
#[case::above_the_baseline(Some(8192))]
fn a_limit_that_reaches_the_baseline_adds_no_cap(#[case] limit: Option<i64>) {
    let mut data = request(UNCAPPED);
    apply_implicit_output_cap(
        &mut data,
        limit,
        Some("completion"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(UNCAPPED));
}

#[test]
fn just_below_the_baseline_still_adds_the_cap() {
    let mut data = request(UNCAPPED);
    apply_implicit_output_cap(
        &mut data,
        Some(4095),
        Some("completion"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":1023}"#));
}

#[rstest]
#[case::explicit(r#"{"max_tokens":10}"#)]
#[case::explicit_zero(r#"{"max_tokens":0}"#)]
#[case::explicit_text(r#"{"max_tokens":"10"}"#)]
fn an_explicit_cap_is_left_alone(#[case] body: &str) {
    let mut data = request(body);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(body));
}

#[test]
fn an_embedding_request_gets_no_cap() {
    let mut data = request(r#"{"input":"hi"}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("embedding"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"input":"hi"}"#));
}

#[test]
fn a_decisions_endpoint_gets_no_cap() {
    let mut data = request(UNCAPPED);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        None,
        CapEndpoint::Decisions,
    );
    assert_eq!(data, request(UNCAPPED));
}

#[test]
fn a_responses_request_at_the_baseline_is_left_alone() {
    let mut data = request(r#"{"input":"hi"}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(4096),
        Some("responses"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"input":"hi"}"#));
}
