#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{DEFAULT_API_BASE, initialize_panw_prisma_airs};
use crate::guardrail_initializers::error::InitializerError;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(extras: &str) -> Result<super::PanwArgs, InitializerError> {
    let body = format!(r#"{{"guardrail": "panw_prisma_airs", "mode": "pre_call"{extras}}}"#);
    initialize_panw_prisma_airs(&params(&body), Some("panw-check"))
}

#[test]
fn panw_refuses_a_missing_api_key_first() {
    let both_missing = build("").unwrap_err();
    assert!(matches!(both_missing, InitializerError::MissingApiKey));
    assert_eq!(
        both_missing.to_string(),
        "PANW Prisma AIRS: api_key is required"
    );
    let profile_configured = build(r#", "profile_name": "profile""#).unwrap_err();
    assert!(matches!(
        profile_configured,
        InitializerError::MissingApiKey
    ));
}

#[test]
fn panw_refuses_an_empty_api_key() {
    let error = build(r#", "api_key": "", "profile_name": "profile""#).unwrap_err();
    assert!(matches!(error, InitializerError::MissingApiKey));
}

#[test]
fn panw_refuses_a_missing_or_empty_profile_name() {
    let missing = build(r#", "api_key": "key""#).unwrap_err();
    assert!(matches!(missing, InitializerError::MissingProfileName));
    assert_eq!(
        missing.to_string(),
        "PANW Prisma AIRS: profile_name is required"
    );
    let empty = build(r#", "api_key": "key", "profile_name": """#).unwrap_err();
    assert!(matches!(empty, InitializerError::MissingProfileName));
}

#[test]
fn panw_args_copy_the_configured_values() {
    let built = build(
        r#",
        "api_key": "panw-key",
        "api_base": "https://panw.example/scan",
        "profile_name": "profile",
        "default_on": true,
        "mask_on_block": true,
        "mask_request_content": true,
        "mask_response_content": true,
        "app_name": "gateway",
        "fallback_on_error": "allow",
        "timeout": 3.5,
        "violation_message_template": "blocked"
        "#,
    )
    .unwrap();
    assert_eq!(built.name, "panw-check");
    assert_eq!(built.api_key.expose_secret(), "panw-key");
    assert_eq!(built.api_base, "https://panw.example/scan");
    assert_eq!(built.profile_name, "profile");
    assert!(built.default_on);
    assert!(built.mask_on_block);
    assert!(built.mask_request_content);
    assert!(built.mask_response_content);
    assert_eq!(built.app_name.as_deref(), Some("gateway"));
    assert_eq!(built.fallback_on_error, FallbackAction::Allow);
    assert_eq!(Some(built.timeout), Some(3.5_f64));
    assert_eq!(built.violation_message_template.as_deref(), Some("blocked"));
}

#[test]
fn panw_applies_the_api_base_timeout_name_and_flag_defaults() {
    let built = build(r#", "api_key": "panw-key", "profile_name": "profile""#).unwrap();
    assert_eq!(built.api_base, DEFAULT_API_BASE);
    assert_eq!(Some(built.timeout), Some(10.0_f64));
    assert_eq!(built.name, "panw-check");
    assert!(!built.mask_on_block);
    assert!(!built.mask_request_content);
    assert!(!built.mask_response_content);
    assert_eq!(built.app_name, None);
    assert_eq!(built.fallback_on_error, FallbackAction::Block);
    assert!(!built.default_on);
    assert_eq!(built.violation_message_template, None);
}

#[test]
fn panw_api_base_and_name_fall_back_when_absent_or_empty() {
    let empty_base =
        build(r#", "api_key": "panw-key", "profile_name": "profile", "api_base": """#).unwrap();
    assert_eq!(empty_base.api_base, DEFAULT_API_BASE);
    let unnamed = initialize_panw_prisma_airs(
        &params(
            r#"{"guardrail": "panw_prisma_airs", "mode": "pre_call", "api_key": "panw-key", "profile_name": "profile"}"#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(unnamed.name, "panw_prisma_airs");
}

#[rstest]
#[case::unknown_fallback(r#", "api_key": "k", "profile_name": "p", "fallback_on_error": "ignore""#)]
#[case::flag_wrong_type(r#", "api_key": "k", "profile_name": "p", "mask_on_block": "yes""#)]
#[case::app_name_wrong_type(r#", "api_key": "k", "profile_name": "p", "app_name": 7"#)]
fn bad_panw_extras_are_refused(#[case] extras: &str) {
    assert!(matches!(
        build(extras),
        Err(InitializerError::InvalidParam(_name))
    ));
}
