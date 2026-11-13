use std::sync::{Mutex, PoisonError};

use betterlite_context::non_default_params::Params;
use rstest::rstest;

use super::support::params;
use crate::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};
use crate::api::responses_parameters::optional::{
    SupportedParamsRequest, check_supported_params, optional_params_responses,
};
use crate::api::responses_parameters::provider::{DropPolicy, ResponsesParamProvider};

#[derive(Debug, Clone, PartialEq)]
struct MappingCall {
    model: String,
    drop_params: bool,
    params: Params,
}

struct Fixture {
    name: String,
    supported: Option<Vec<String>>,
    mapped: Option<Params>,
    omit: Vec<String>,
    calls: Mutex<Vec<MappingCall>>,
}

impl Fixture {
    fn new(supported: Option<Vec<String>>) -> Self {
        Self {
            name: "openai".to_owned(),
            supported,
            mapped: None,
            omit: Vec::new(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn mapped(mut self, source: &str) -> Self {
        self.mapped = Some(params(source));
        self
    }

    fn calls(&self) -> Vec<MappingCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponsesParamProvider for Fixture {
    type Error = ResponsesParametersError;

    fn name(&self) -> &str {
        &self.name
    }

    fn supported_params(&self, _model: &str) -> Result<Option<Vec<String>>, Self::Error> {
        Ok(self.supported.clone())
    }

    fn map_params(
        &self,
        params: &Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(MappingCall {
            model: model.to_owned(),
            drop_params,
            params: params.clone(),
        });
        drop(calls);
        let mut mapped = self.mapped.clone().unwrap_or_else(|| params.clone());
        for name in &self.omit {
            mapped.remove(name);
        }
        Ok(mapped)
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn unsupported_params_raise_before_mapping_with_both_flags_off() {
    let fixture = Fixture::new(Some(names(&["store"])));
    let mut request = params(r#"{"foo": 1}"#);
    let error = optional_params_responses(&fixture, "m", &mut request, &[], DropPolicy::default())
        .expect_err("an unsupported parameter must fail");
    let ResponsesParametersError::UnsupportedParams {
        provider,
        model,
        params: unsupported,
    } = error
    else {
        panic!("the unsupported parameter error is missing");
    };
    assert_eq!(provider, "openai");
    assert_eq!(model, "m");
    assert_eq!(unsupported, "foo");
    assert_eq!(UNSUPPORTED_PARAMS_STATUS, 400);
    assert!(fixture.calls().is_empty(), "validation precedes mapping");
    assert_eq!(
        request,
        params(r#"{"foo": 1}"#),
        "the request map is unchanged"
    );
}

#[rstest]
#[case::global_true(true, None, true)]
#[case::call_true(false, Some(true), true)]
#[case::call_false_cannot_disable_global(true, Some(false), true)]
fn either_true_drop_flag_reaches_the_mapper(
    #[case] global: bool,
    #[case] call: Option<bool>,
    #[case] expected: bool,
) {
    let fixture = Fixture::new(Some(names(&["store"])));
    let mut request = params(r#"{"foo": 1}"#);
    let policy = DropPolicy { global, call };
    optional_params_responses(&fixture, "m", &mut request, &[], policy).expect("dropping admits");
    let calls = fixture.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].drop_params, expected);
    assert_eq!(calls[0].model, "m");
}

#[test]
fn a_false_call_flag_without_global_dropping_still_raises() {
    let fixture = Fixture::new(Some(names(&["store"])));
    let mut request = params(r#"{"foo": 1}"#);
    let policy = DropPolicy {
        global: false,
        call: Some(false),
    };
    let result = optional_params_responses(&fixture, "m", &mut request, &[], policy);
    assert!(matches!(
        result,
        Err(ResponsesParametersError::UnsupportedParams { .. })
    ));
    assert!(fixture.calls().is_empty());
}

#[test]
fn null_supported_skips_validation() {
    let request = params(r#"{"foo": 1}"#);
    check_supported_params(&SupportedParamsRequest {
        provider: "openai",
        model: "m",
        supported: None,
        params: &request,
        allowed: &[],
        drop_params: false,
    })
    .expect("a null supported list skips validation");
}

#[test]
fn allowed_params_are_restored_from_the_requested_map() {
    let fixture = Fixture::new(Some(names(&["store"]))).mapped("{}");
    let mut request = params(r#"{"foo": 1}"#);
    let mapped = optional_params_responses(
        &fixture,
        "m",
        &mut request,
        &names(&["foo"]),
        DropPolicy::default(),
    )
    .expect("an allowed parameter is admitted");
    assert_eq!(mapped, params(r#"{"foo": 1}"#));
    assert!(
        !request.contains_key("foo"),
        "restoration pops the parameter from the requested map"
    );
}

#[test]
fn a_mapped_value_is_not_overwritten() {
    let fixture = Fixture::new(Some(names(&["store"]))).mapped(r#"{"foo": 2}"#);
    let mut request = params(r#"{"foo": 1}"#);
    let mapped = optional_params_responses(
        &fixture,
        "m",
        &mut request,
        &names(&["foo"]),
        DropPolicy::default(),
    )
    .expect("an allowed parameter is admitted");
    assert_eq!(mapped, params(r#"{"foo": 2}"#));
    assert_eq!(request, params(r#"{"foo": 1}"#));
}

#[test]
fn an_absent_allowed_field_is_not_invented() {
    let fixture = Fixture::new(Some(names(&["store", "foo"]))).mapped("{}");
    let mut request = params(r#"{"store": true}"#);
    let mapped = optional_params_responses(
        &fixture,
        "m",
        &mut request,
        &names(&["foo"]),
        DropPolicy::default(),
    )
    .expect("an allowed parameter is admitted");
    assert_eq!(mapped, params("{}"));
    assert_eq!(request, params(r#"{"store": true}"#));
}

#[test]
fn the_mapper_receives_the_requested_map_before_overrides() {
    let fixture = Fixture::new(Some(names(&["store", "foo"]))).mapped("{}");
    let mut request = params(r#"{"foo": 1, "store": false}"#);
    optional_params_responses(
        &fixture,
        "m",
        &mut request,
        &names(&["foo"]),
        DropPolicy::default(),
    )
    .expect("an allowed parameter is admitted");
    let calls = fixture.calls();
    assert_eq!(calls[0].params, params(r#"{"foo": 1, "store": false}"#));
}

#[rstest]
#[case::boolean_true(
    r#"{"stream_options": {"include_obfuscation": true}}"#,
    r#"{"stream_options": {"include_obfuscation": true}}"#
)]
#[case::boolean_false(
    r#"{"stream_options": {"include_obfuscation": false}}"#,
    r#"{"stream_options": {"include_obfuscation": false}}"#
)]
#[case::empty_object(r#"{"stream_options": {}}"#, "{}")]
#[case::include_usage(r#"{"stream_options": {"include_usage": true}}"#, "{}")]
#[case::integer_flag(r#"{"stream_options": {"include_obfuscation": 1}}"#, "{}")]
#[case::text_flag(r#"{"stream_options": {"include_obfuscation": "true"}}"#, "{}")]
fn stream_options_are_normalized_after_mapping(#[case] source: &str, #[case] expected: &str) {
    let fixture = Fixture::new(None).mapped(source);
    let mut request = Params::new();
    let mapped = optional_params_responses(&fixture, "m", &mut request, &[], DropPolicy::default())
        .expect("null supported skips validation");
    assert_eq!(mapped, params(expected));
}

#[test]
fn a_malformed_stream_options_is_removed_from_the_mapped_output() {
    let fixture = Fixture::new(None).mapped(r#"{"store": true, "stream_options": {}}"#);
    let mut request = Params::new();
    let mapped = optional_params_responses(&fixture, "m", &mut request, &[], DropPolicy::default())
        .expect("null supported skips validation");
    assert_eq!(mapped, params(r#"{"store": true}"#));
}

#[test]
fn an_allowed_malformed_stream_options_is_restored_after_normalization() {
    let fixture = Fixture::new(None).mapped("{}");
    let mut request = params(r#"{"stream_options": {"include_obfuscation": 1}}"#);
    let mapped = optional_params_responses(
        &fixture,
        "m",
        &mut request,
        &names(&["stream_options"]),
        DropPolicy::default(),
    )
    .expect("null supported skips validation");
    assert_eq!(
        mapped,
        params(r#"{"stream_options": {"include_obfuscation": 1}}"#)
    );
    assert!(request.is_empty(), "restoration pops stream_options");
}

#[test]
fn a_malformed_stream_options_is_dropped_when_it_is_not_allowed() {
    let fixture = Fixture::new(None);
    let mut request = params(r#"{"stream_options": {"include_obfuscation": "true"}}"#);
    let mapped = optional_params_responses(&fixture, "m", &mut request, &[], DropPolicy::default())
        .expect("null supported skips validation");
    assert!(mapped.is_empty());
}
