#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::CallState;
use super::merge_response_hidden_params;
use super::surface_response_headers;
use crate::logging_metadata::params::LitellmParams;

fn hidden(encoded: &str) -> Json {
    serde_json::from_str(encoded).expect("hidden params decode")
}

fn params(encoded: &str) -> LitellmParams {
    serde_json::from_str(encoded).expect("params decode")
}

fn headers(encoded: &str) -> IndexMap<String, String> {
    serde_json::from_str(encoded).expect("headers decode")
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn response_headers_are_taken_from_the_hidden_params() {
    let mut state = CallState::default();
    surface_response_headers(
        &mut state,
        Some(&hidden(r#"{"headers": {"x-request-id": "req-1"}}"#)),
    );
    assert_eq!(
        state.response_headers,
        Some(headers(r#"{"x-request-id": "req-1"}"#))
    );
}

#[test]
fn existing_response_headers_are_never_replaced() {
    let mut state = CallState {
        response_headers: Some(headers(r#"{"kept": "yes"}"#)),
        litellm_params: None,
        response_cost: None,
    };
    surface_response_headers(
        &mut state,
        Some(&hidden(r#"{"headers": {"x-request-id": "req-1"}}"#)),
    );
    assert_eq!(state.response_headers, Some(headers(r#"{"kept": "yes"}"#)));
}

#[rstest]
#[case::absent(None)]
#[case::empty_object(Some("{}"))]
#[case::no_headers(Some(r#"{"response_cost": 1}"#))]
#[case::null_headers(Some(r#"{"headers": null}"#))]
#[case::non_object_headers(Some(r#"{"headers": ["x"]}"#))]
#[case::non_text_values(Some(r#"{"headers": {"x-request-id": 7}}"#))]
#[case::not_an_object(Some("[1]"))]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: Option<&str>) {
    let mut state = CallState::default();
    surface_response_headers(&mut state, encoded.map(hidden).as_ref());
    assert_eq!(state.response_headers, None);
}

#[test]
fn hidden_params_are_merged_into_metadata_and_keep_a_recorded_cost() {
    let mut state = CallState {
        response_headers: None,
        litellm_params: Some(params(r#"{"metadata": {"kept": true}}"#)),
        response_cost: Some(number(9)),
    };
    merge_response_hidden_params(
        &mut state,
        Some(&hidden(r#"{"cache_key": "c", "response_cost": 1}"#)),
    );
    let merged = state
        .litellm_params
        .expect("params")
        .metadata
        .expect("metadata")
        .shift_remove("hidden_params")
        .expect("hidden params");
    let Json::Object(fields) = merged else {
        panic!("hidden params are an object");
    };
    assert_eq!(fields.get("cache_key"), Some(&Json::Text("c".to_owned())));
    assert_eq!(fields.get("response_cost"), Some(&number(1)));
}

#[test]
fn a_missing_or_null_hidden_cost_is_filled_from_the_call_details() {
    let mut state = CallState {
        response_headers: None,
        litellm_params: Some(params("{}")),
        response_cost: Some(number(9)),
    };
    merge_response_hidden_params(&mut state, Some(&hidden(r#"{"response_cost": null}"#)));
    let merged = state
        .litellm_params
        .expect("params")
        .metadata
        .expect("metadata")
        .shift_remove("hidden_params")
        .expect("hidden params");
    let Json::Object(fields) = merged else {
        panic!("hidden params are an object");
    };
    assert_eq!(fields.get("response_cost"), Some(&number(9)));
}

#[test]
fn empty_hidden_params_are_not_merged() {
    let mut state = CallState {
        response_headers: None,
        litellm_params: Some(params(r#"{"metadata": {"kept": true}}"#)),
        response_cost: Some(number(9)),
    };
    merge_response_hidden_params(&mut state, Some(&hidden("{}")));
    let metadata = state
        .litellm_params
        .expect("params")
        .metadata
        .expect("metadata");
    assert!(!metadata.contains_key("hidden_params"));
}

#[test]
fn hidden_params_are_not_merged_without_litellm_params() {
    let mut state = CallState {
        response_headers: None,
        litellm_params: None,
        response_cost: None,
    };
    merge_response_hidden_params(&mut state, Some(&hidden(r#"{"cache_key": "c"}"#)));
    assert!(state.litellm_params.is_none());
}

#[test]
fn absent_hidden_params_are_not_merged() {
    let mut state = CallState {
        response_headers: None,
        litellm_params: Some(params("{}")),
        response_cost: None,
    };
    merge_response_hidden_params(&mut state, None);
    assert!(state.litellm_params.expect("params").metadata.is_none());
}
