use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::fixtures::{call_kwargs, json, object, payload, sources};
use crate::fields::payload::StandardLoggingPayload;
use crate::newrelic_extraction::request_params::{RequestParameters, from_sources};

fn params_of(
    standard: Option<&StandardLoggingPayload>,
    optional: Option<&IndexMap<String, Json>>,
) -> RequestParameters {
    let body = object("{}");
    let mut kwargs = call_kwargs();
    kwargs.optional_params = optional;
    let event_sources = sources(&kwargs, &body, standard);
    from_sources(&event_sources)
}

#[test]
fn newrelic_extraction_tests_behavior_request_params_reads_the_payload() {
    let mut standard = payload();
    standard.model_parameters = object(r#"{"temperature":0.5,"max_tokens":100}"#);
    let parameters = params_of(Some(&standard), None);
    assert_eq!(parameters.temperature, Some(json("0.5")));
    assert_eq!(parameters.max_tokens, Some(json("100")));
}

#[test]
fn newrelic_extraction_tests_behavior_request_params_prefers_the_payload() {
    let mut standard = payload();
    standard.model_parameters = object(r#"{"temperature":0.5}"#);
    let optional = object(r#"{"temperature":0.9,"max_tokens":20}"#);
    let parameters = params_of(Some(&standard), Some(&optional));
    assert_eq!(parameters.temperature, Some(json("0.5")));
    assert_eq!(parameters.max_tokens, None);
}

#[test]
fn newrelic_extraction_tests_behavior_request_params_falls_back_to_optional_params() {
    let optional = object(r#"{"temperature":0.9,"max_tokens":20}"#);
    let parameters = params_of(None, Some(&optional));
    assert_eq!(parameters.temperature, Some(json("0.9")));
    assert_eq!(parameters.max_tokens, Some(json("20")));
}

#[test]
fn newrelic_extraction_tests_behavior_request_params_omits_absent_and_null_values() {
    let optional = object(r#"{"temperature":null}"#);
    let parameters = params_of(None, Some(&optional));
    assert_eq!(parameters, RequestParameters::default());
}

#[test]
fn newrelic_extraction_tests_behavior_request_params_defaults_without_any_source() {
    assert_eq!(params_of(None, None), RequestParameters::default());
}

#[rstest]
#[case::zero(r#"{"temperature":0,"max_tokens":0}"#, "0", "0")]
#[case::empty_string(r#"{"temperature":"","max_tokens":""}"#, r#""""#, r#""""#)]
#[case::empty_list(r#"{"temperature":[],"max_tokens":[]}"#, "[]", "[]")]
fn newrelic_extraction_tests_behavior_request_params_keeps_explicit_values(
    #[case] encoded: &str,
    #[case] temperature: &str,
    #[case] max_tokens: &str,
) {
    let optional = object(encoded);
    let parameters = params_of(None, Some(&optional));
    assert_eq!(parameters.temperature, Some(json(temperature)));
    assert_eq!(parameters.max_tokens, Some(json(max_tokens)));
}
