use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{Headers, Object};
use rstest::rstest;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::json;

use super::super::error::PrepareError;
use super::super::{PrepareInput, prepare};

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the case params are a JSON object")
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn empty_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn prepares_with_an_explicit_key() {
    let params = params(r#"{"soniox_polling_interval": 2}"#);
    let environment = empty_environment();

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: Some("explicit"),
        api_base: None,
        environment: &environment,
    })
    .expect("the key resolves");

    assert_eq!(
        prepared
            .auth_headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer explicit")
    );
    assert_eq!(prepared.api_base, "https://api.soniox.com");
    assert_eq!(prepared.options.poll_interval.to_f64(), Some(2.0_f64));
    assert!(!prepared.params.contains_key("soniox_polling_interval"));
}

#[test]
fn reads_the_environment_key_and_base() {
    let params = Object::new();
    let environment = environment(&[
        ("SONIOX_API_KEY", "environment"),
        ("SONIOX_API_BASE", "https://env.test///"),
    ]);

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: None,
        api_base: None,
        environment: &environment,
    })
    .expect("the environment is loaded");

    assert_eq!(
        prepared
            .auth_headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer environment")
    );
    assert_eq!(prepared.api_base, "https://env.test");
}

#[test]
fn prefers_the_explicit_base_over_the_environment() {
    let params = Object::new();
    let environment = environment(&[("SONIOX_API_BASE", "https://env.test")]);

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: Some("explicit"),
        api_base: Some("https://explicit.test/path/"),
        environment: &environment,
    })
    .expect("the base resolves");

    assert_eq!(prepared.api_base, "https://explicit.test/path");
}

#[test]
fn keeps_a_caller_authorization_header() {
    let params = Object::new();
    let environment = empty_environment();
    let headers = Headers::from([(String::from("Authorization"), String::from("Bearer caller"))]);

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers,
        api_key: Some("explicit"),
        api_base: None,
        environment: &environment,
    })
    .expect("the headers merge");

    assert_eq!(
        prepared
            .auth_headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn refuses_a_missing_key_before_it_reads_options() {
    let params = params(r#"{"soniox_polling_interval": "abc"}"#);
    let environment = empty_environment();

    let error = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: None,
        api_base: None,
        environment: &environment,
    })
    .expect_err("the key is missing");

    assert!(matches!(error, PrepareError::Auth(ref auth) if auth.status_code == 401));
}

#[rstest]
#[case::empty("")]
#[case::true_flag("true")]
#[case::false_flag("false")]
fn treats_an_unusable_environment_key_as_missing(#[case] value: &str) {
    let params = Object::new();
    let environment = environment(&[("SONIOX_API_KEY", value)]);

    let error = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: None,
        api_base: None,
        environment: &environment,
    })
    .expect_err("the key is unset");

    assert!(matches!(error, PrepareError::Auth(ref auth) if auth.status_code == 401));
}

#[rstest]
#[case::true_flag("true")]
#[case::false_flag("false")]
fn treats_a_boolean_environment_base_as_the_default(#[case] value: &str) {
    let params = Object::new();
    let environment = environment(&[("SONIOX_API_KEY", "explicit"), ("SONIOX_API_BASE", value)]);

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: None,
        api_base: None,
        environment: &environment,
    })
    .expect("the key resolves");

    assert_eq!(prepared.api_base, "https://api.soniox.com");
}

#[test]
fn leaves_the_callers_params_untouched() {
    let params = params(
        r#"{"soniox_polling_interval": 2, "language": "en", "filename": "clip.wav", "context": {"topic": "a"}, "client_reference_id": null}"#,
    );
    let environment = empty_environment();

    let prepared = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: Some("explicit"),
        api_base: None,
        environment: &environment,
    })
    .expect("the params are accepted");

    assert_eq!(params.get("soniox_polling_interval"), Some(&json!(2)));
    assert!(params.contains_key("language"));
    assert!(params.contains_key("filename"));
    assert!(!prepared.params.contains_key("language"));
    assert!(!prepared.params.contains_key("filename"));
    assert!(prepared.params.contains_key("context"));
    assert!(prepared.params.contains_key("client_reference_id"));
}

#[test]
fn names_the_field_an_option_fails_on() {
    let params = params(r#"{"soniox_cleanup": [1]}"#);
    let environment = empty_environment();

    let error = prepare(&PrepareInput {
        optional_params: &params,
        headers: Headers::new(),
        api_key: Some("explicit"),
        api_base: None,
        environment: &environment,
    })
    .expect_err("the cleanup entry is refused");

    assert!(error.to_string().contains("soniox_cleanup"), "{error}");
}
