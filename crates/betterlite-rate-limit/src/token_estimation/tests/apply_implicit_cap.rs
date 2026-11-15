#![cfg(test)]

use rstest::rstest;

use super::output_cap::{CapEndpoint, apply_implicit_output_cap};
use super::support::request;

#[test]
fn a_small_limit_caps_an_uncapped_chat_request() {
    let mut data = request(r#"{"messages":[]}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":250}"#));
}

#[test]
fn without_a_call_type_the_cap_is_written_to_max_tokens() {
    let mut data = request(r#"{"messages":[]}"#);
    apply_implicit_output_cap(&mut data, Some(1000), None, None, CapEndpoint::Generic);
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":250}"#));
}

#[test]
fn a_configured_output_estimate_raises_the_cap() {
    let mut data = request(r#"{"messages":[]}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        Some(500),
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":500}"#));
}

#[test]
fn a_configured_output_estimate_below_the_floor_keeps_the_floor() {
    let mut data = request(r#"{"messages":[]}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        Some(100),
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":250}"#));
}

#[rstest]
#[case::text(r#"{"messages":[],"max_tokens":"wide"}"#)]
#[case::boolean(r#"{"messages":[],"max_tokens":true}"#)]
#[case::list(r#"{"messages":[],"max_tokens":[5]}"#)]
#[case::object(r#"{"messages":[],"max_tokens":{"n":5}}"#)]
fn a_non_numeric_output_field_is_left_untouched(#[case] body: &str) {
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
fn a_null_output_field_counts_as_absent() {
    let mut data = request(r#"{"messages":[],"max_tokens":null}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("completion"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"messages":[],"max_tokens":250}"#));
}

#[test]
fn a_responses_request_is_capped_through_max_output_tokens() {
    let mut data = request(r#"{"input":"hi"}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("responses"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"input":"hi","max_output_tokens":250}"#));
}

#[test]
fn a_responses_request_never_goes_below_the_sixteen_token_minimum() {
    let mut data = request(r#"{"input":"hi"}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(64),
        Some("aresponses"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"input":"hi","max_output_tokens":16}"#));
}

#[test]
fn a_google_request_without_a_config_gets_one() {
    let mut data = request(r#"{"contents":[{"parts":[{"text":"hi"}]}]}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("generate_content"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(
        data,
        request(r#"{"contents":[{"parts":[{"text":"hi"}]}],"config":{"maxOutputTokens":250}}"#)
    );
}

#[test]
fn a_google_request_with_only_a_generation_config_keeps_its_spelling() {
    let mut data = request(r#"{"contents":[],"generationConfig":{"temperature":1}}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("generate_content"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(
        data,
        request(r#"{"contents":[],"generationConfig":{"temperature":1,"maxOutputTokens":250}}"#)
    );
}

#[test]
fn a_google_request_with_both_config_spellings_writes_config() {
    let mut data = request(r#"{"contents":[],"config":{},"generationConfig":{}}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("agenerate_content"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(
        data,
        request(r#"{"contents":[],"config":{"maxOutputTokens":250},"generationConfig":{}}"#)
    );
}

#[test]
fn a_google_request_with_a_non_mapping_config_is_left_alone() {
    let mut data = request(r#"{"contents":[],"config":"none"}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("generate_content"),
        None,
        CapEndpoint::Generic,
    );
    assert_eq!(data, request(r#"{"contents":[],"config":"none"}"#));
}

#[test]
fn an_unparsable_google_cap_is_replaced() {
    let mut data =
        request(r#"{"contents":[],"config":{"maxOutputTokens":"wide","temperature":1}}"#);
    apply_implicit_output_cap(
        &mut data,
        Some(1000),
        Some("generate_content"),
        Some(600),
        CapEndpoint::Generic,
    );
    assert_eq!(
        data,
        request(r#"{"contents":[],"config":{"maxOutputTokens":600,"temperature":1}}"#)
    );
}
