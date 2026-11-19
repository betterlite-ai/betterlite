#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use crate::params::{
    CallType, ChatParameters, ParameterError, ParameterPolicy, map_listed_parameters,
};

const SUPPORTED: &[&str] = &["temperature"];

fn parameters(source: &str) -> ChatParameters {
    serde_json::from_str(source).unwrap()
}

fn policy(source: &str) -> ParameterPolicy {
    serde_json::from_str(source).unwrap()
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn unsupported_rejected() {
    let parameters = parameters(r#"{"reasoning_effort": "low"}"#);
    let error = map_listed_parameters(
        SUPPORTED,
        "claude-3",
        CallType::ChatCompletion,
        &parameters,
        &policy("{}"),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ParameterError::Unsupported { model, names }
            if model == "claude-3" && names == ["reasoning_effort".to_owned()]
    ));
}

fn drop_params_drops() {
    let parameters = parameters(r#"{"reasoning_effort": "low"}"#);
    let mapping = map_listed_parameters(
        SUPPORTED,
        "claude-3",
        CallType::ChatCompletion,
        &parameters,
        &policy(r#"{"drop_params": true}"#),
    )
    .unwrap();
    assert_eq!(mapping.dropped_fields, ["reasoning_effort"]);
    assert!(mapping.standard.is_empty());
}

fn allowed_openai_params_pass() {
    let parameters = parameters(r#"{"reasoning_effort": "low"}"#);
    let mapping = map_listed_parameters(
        &[],
        "claude-3",
        CallType::ChatCompletion,
        &parameters,
        &policy(r#"{"allowed_openai_params": ["reasoning_effort"]}"#),
    )
    .unwrap();
    assert_eq!(mapping.standard["reasoning_effort"], value(r#""low""#));
    assert!(mapping.dropped_fields.is_empty());
}

fn provider_field_passes() {
    let parameters = parameters(r#"{"vendor_flag": {"mode": "fast"}}"#);
    let mapping = map_listed_parameters(
        &[],
        "claude-3",
        CallType::ChatCompletion,
        &parameters,
        &policy("{}"),
    )
    .unwrap();
    assert_eq!(
        mapping.provider_fields["vendor_flag"],
        value(r#"{"mode": "fast"}"#)
    );
    assert!(mapping.standard.is_empty());
}

#[rstest]
#[case::unsupported_rejected(unsupported_rejected)]
#[case::drop_params_drops(drop_params_drops)]
#[case::allowed_openai_params_pass(allowed_openai_params_pass)]
#[case::provider_field_passes(provider_field_passes)]
fn listed_parameter_cases(#[case] scenario: fn()) {
    scenario();
}
