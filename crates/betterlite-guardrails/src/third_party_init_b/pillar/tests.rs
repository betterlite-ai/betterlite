#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_pillar;
use crate::third_party_init_b::error::ThirdPartyInitBError;

fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "pillar", "mode": "pre_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn pillar_refuses_a_missing_or_empty_name() {
    let missing = initialize_pillar(&params(""), None).unwrap_err();
    assert!(matches!(missing, ThirdPartyInitBError::MissingPillarName));
    assert_eq!(missing.to_string(), "Pillar guardrail name is required");
    let empty = initialize_pillar(&params(""), Some("")).unwrap_err();
    assert!(matches!(empty, ThirdPartyInitBError::MissingPillarName));
}

#[test]
fn pillar_applies_the_defaults() {
    let built = initialize_pillar(&params(""), Some("pillar-check")).unwrap();
    assert_eq!(built.name, "pillar-check");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.on_flagged_action.as_deref(), Some("monitor"));
    assert_eq!(built.async_mode, None);
    assert_eq!(built.persist_session, None);
    assert_eq!(built.include_scanners, Some(true));
    assert_eq!(built.include_evidence, Some(true));
    assert_eq!(built.fallback_on_error, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn pillar_copies_the_config_values() {
    let built = initialize_pillar(
        &params(
            r#",
            "api_key": "pillar-key",
            "api_base": "https://pillar.test",
            "on_flagged_action": "mask",
            "async_mode": true,
            "persist_session": false,
            "include_scanners": false,
            "include_evidence": false,
            "fallback_on_error": "allow",
            "timeout": 3.5,
            "default_on": true
            "#,
        ),
        Some("pillar-check"),
    )
    .unwrap();
    assert_eq!(
        built
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("pillar-key".to_owned())
    );
    assert_eq!(built.api_base.as_deref(), Some("https://pillar.test"));
    assert_eq!(built.on_flagged_action.as_deref(), Some("mask"));
    assert_eq!(built.async_mode, Some(true));
    assert_eq!(built.persist_session, Some(false));
    assert_eq!(built.include_scanners, Some(false));
    assert_eq!(built.include_evidence, Some(false));
    assert_eq!(built.fallback_on_error.as_deref(), Some("allow"));
    assert_eq!(built.timeout, Some(3.5_f64));
    assert!(built.default_on);
}

#[test]
fn pillar_optional_params_win_over_the_top_level_param() {
    let built = initialize_pillar(
        &params(
            r#",
            "async_mode": false,
            "timeout": 9,
            "fallback_on_error": "block",
            "include_scanners": true,
            "include_evidence": false,
            "optional_params": {
                "async_mode": true,
                "timeout": 2.5,
                "fallback_on_error": "allow",
                "include_scanners": false,
                "include_evidence": null
            }
            "#,
        ),
        Some("pillar-check"),
    )
    .unwrap();
    assert_eq!(built.async_mode, Some(true));
    assert_eq!(built.timeout, Some(2.5_f64));
    assert_eq!(built.fallback_on_error.as_deref(), Some("allow"));
    assert_eq!(built.include_scanners, Some(false));
    assert_eq!(built.include_evidence, Some(false));
}

#[test]
fn pillar_optional_params_drop_null_entries() {
    let built = initialize_pillar(
        &params(
            r#",
            "async_mode": false,
            "persist_session": true,
            "timeout": 9,
            "fallback_on_error": "block",
            "optional_params": {
                "async_mode": null,
                "persist_session": null,
                "timeout": null,
                "fallback_on_error": null
            }
            "#,
        ),
        Some("pillar-check"),
    )
    .unwrap();
    assert_eq!(built.async_mode, Some(false));
    assert_eq!(built.persist_session, Some(true));
    assert_eq!(built.timeout, Some(9.0_f64));
    assert_eq!(built.fallback_on_error.as_deref(), Some("block"));
}

#[test]
fn pillar_keeps_an_explicit_null_at_the_top_level() {
    let built = initialize_pillar(
        &params(r#", "on_flagged_action": null, "include_scanners": null"#),
        Some("pillar-check"),
    )
    .unwrap();
    assert_eq!(built.on_flagged_action, None);
    assert_eq!(built.include_scanners, None);
    assert_eq!(built.include_evidence, Some(true));
}

#[rstest]
#[case::scalar(r#", "optional_params": "text""#)]
#[case::wrong_async_mode(r#", "optional_params": {"async_mode": "yes"}"#)]
#[case::wrong_timeout(r#", "optional_params": {"timeout": "slow"}"#)]
fn pillar_refuses_a_bad_optional_params(#[case] extras: &str) {
    let error = initialize_pillar(&params(extras), Some("pillar-check")).unwrap_err();
    assert!(matches!(error, ThirdPartyInitBError::InvalidParam { .. }));
}
