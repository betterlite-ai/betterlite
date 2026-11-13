#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::{SingulrInitError, initialize_singulr};

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(extras: &str) -> super::SingulrArgs {
    let body = format!(r#"{{"guardrail": "singulr", "mode": "post_call"{extras}}}"#);
    initialize_singulr(&params(&body), Some("singulr-check")).unwrap()
}

#[test]
fn the_initializer_copies_every_singulr_field() {
    let built = build(
        r#",
        "singulr_api_key": "singulr-key",
        "singulr_api_base": "https://singulr.example",
        "singulr_application_id": "entity-1",
        "singulr_guardrail_id": "guardrail-9",
        "block_on_error": false,
        "timeout": 4.0,
        "default_on": true
        "#,
    );
    assert_eq!(built.name, "singulr-check");
    assert_eq!(
        built.singulr_api_key.as_ref().unwrap().expose_secret(),
        "singulr-key"
    );
    assert_eq!(
        built.singulr_api_base.as_deref(),
        Some("https://singulr.example")
    );
    assert_eq!(built.singulr_application_id.as_deref(), Some("entity-1"));
    assert_eq!(built.singulr_guardrail_id.as_deref(), Some("guardrail-9"));
    assert_eq!(built.block_on_error, Some(false));
    assert_eq!(built.timeout, Some(4.0_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("post_call".to_owned())
    );
}

#[test]
fn a_missing_singulr_api_base_falls_back_to_the_shared_field() {
    let built = build(r#", "api_base": "https://shared.example""#);
    assert_eq!(
        built.singulr_api_base.as_deref(),
        Some("https://shared.example")
    );
}

#[test]
fn a_missing_singulr_api_key_falls_back_to_the_shared_field() {
    let built = build(r#", "api_key": "shared-key""#);
    assert_eq!(
        built.singulr_api_key.as_ref().unwrap().expose_secret(),
        "shared-key"
    );
}

#[test]
fn an_empty_singulr_api_base_is_used_as_given() {
    let built = build(r#", "singulr_api_base": "", "api_base": "https://shared.example""#);
    assert_eq!(built.singulr_api_base.as_deref(), Some(""));
}

#[test]
fn an_absent_singulr_field_stays_absent() {
    let built = build("");
    assert!(built.singulr_api_key.is_none());
    assert!(built.singulr_api_base.is_none());
    assert!(built.singulr_application_id.is_none());
    assert!(built.singulr_guardrail_id.is_none());
    assert_eq!(built.block_on_error, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
}

#[test]
fn the_name_is_empty_when_the_caller_passes_none() {
    let built = initialize_singulr(
        &params(r#"{"guardrail": "singulr", "mode": "pre_call"}"#),
        None,
    )
    .unwrap();
    assert_eq!(built.name, "");
}

#[test]
fn a_text_field_of_another_type_is_an_error() {
    let body = r#"{"guardrail": "singulr", "mode": "pre_call", "singulr_api_key": 5}"#;
    assert!(matches!(
        initialize_singulr(&params(body), None).unwrap_err(),
        SingulrInitError::Text { .. }
    ));
}

#[test]
fn a_block_on_error_of_another_type_is_an_error() {
    let body = r#"{"guardrail": "singulr", "mode": "pre_call", "block_on_error": "yes"}"#;
    assert!(matches!(
        initialize_singulr(&params(body), None).unwrap_err(),
        SingulrInitError::Flag { .. }
    ));
}
