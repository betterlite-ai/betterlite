#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_promptguard;
use crate::guardrail_initializers::error::InitializerError;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).expect("params")
}

#[test]
fn promptguard_args_copy_every_setting() {
    let body = r#"{
        "guardrail": "promptguard",
        "mode": "post_call",
        "api_base": "https://pg.example",
        "api_key": "pg-key",
        "block_on_error": true,
        "timeout": 3.5,
        "default_on": true
    }"#;
    let built = initialize_promptguard(&params(body), Some("pg-check")).expect("args");
    assert_eq!(built.name, "pg-check");
    assert_eq!(built.api_base.as_deref(), Some("https://pg.example"));
    assert_eq!(
        built.api_key.as_ref().expect("key").expose_secret(),
        "pg-key"
    );
    assert_eq!(built.block_on_error, Some(true));
    assert_eq!(built.timeout, Some(3.5));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn promptguard_args_keep_the_block_on_error_false() {
    let body = r#"{"guardrail": "promptguard", "mode": "pre_call", "block_on_error": false}"#;
    let built = initialize_promptguard(&params(body), None).expect("args");
    assert_eq!(built.block_on_error, Some(false));
}

#[test]
fn promptguard_args_leave_absent_values_absent() {
    let body = r#"{"guardrail": "promptguard", "mode": "pre_call"}"#;
    let built = initialize_promptguard(&params(body), None).expect("args");
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert_eq!(built.api_key, None);
    assert_eq!(built.block_on_error, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn promptguard_nulls_stay_absent() {
    let body = r#"{
        "guardrail": "promptguard",
        "mode": "pre_call",
        "api_base": null,
        "api_key": null,
        "block_on_error": null,
        "timeout": null,
        "default_on": null
    }"#;
    let built = initialize_promptguard(&params(body), Some("pg-check")).expect("args");
    assert_eq!(built.api_base, None);
    assert_eq!(built.api_key, None);
    assert_eq!(built.block_on_error, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
}

#[rstest]
#[case::text("true")]
#[case::integer("1")]
#[case::boolean_list("[]")]
fn a_wrong_block_on_error_names_the_field(#[case] value: &str) {
    let body =
        format!(r#"{{"guardrail": "promptguard", "mode": "pre_call", "block_on_error": {value}}}"#);
    let error = initialize_promptguard(&params(&body), None).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidParam(name) if name == "block_on_error"),
        "unexpected error {error}"
    );
}
