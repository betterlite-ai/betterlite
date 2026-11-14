#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_xecguard;
use crate::third_party_init_b::error::ThirdPartyInitBError;

fn params(extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "xecguard", "mode": "pre_call"{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn xecguard_copies_the_config_values() {
    let built = initialize_xecguard(
        &params(
            r#",
            "api_key": "xec-key",
            "api_base": "https://xec.test",
            "xecguard_model": "xecguard_v3",
            "policy_names": ["one", "two"],
            "block_on_error": false,
            "grounding_strictness": "STRICT",
            "timeout": 8.5,
            "default_on": true
            "#,
        ),
        Some("xec-check"),
    )
    .unwrap();
    assert_eq!(built.name, "xec-check");
    assert_eq!(
        built
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("xec-key".to_owned())
    );
    assert_eq!(built.api_base.as_deref(), Some("https://xec.test"));
    assert_eq!(built.xecguard_model.as_deref(), Some("xecguard_v3"));
    assert_eq!(
        built.policy_names,
        Some(vec!["one".to_owned(), "two".to_owned()])
    );
    assert_eq!(built.block_on_error, Some(false));
    assert_eq!(built.grounding_strictness.as_deref(), Some("STRICT"));
    assert_eq!(built.timeout, Some(8.5_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn xecguard_applies_the_defaults() {
    let built = initialize_xecguard(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.xecguard_model, None);
    assert_eq!(built.policy_names, None);
    assert_eq!(built.block_on_error, None);
    assert_eq!(built.grounding_strictness, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
}

#[rstest]
#[case::balanced("BALANCED")]
#[case::strict("STRICT")]
fn xecguard_keeps_a_known_grounding_strictness(#[case] strictness: &str) {
    let extras = format!(r#", "grounding_strictness": "{strictness}""#);
    let built = initialize_xecguard(&params(&extras), None).unwrap();
    assert_eq!(built.grounding_strictness.as_deref(), Some(strictness));
}

#[rstest]
#[case::lowercase("strict")]
#[case::unknown("RELAXED")]
fn xecguard_refuses_an_unknown_grounding_strictness(#[case] strictness: &str) {
    let extras = format!(r#", "grounding_strictness": "{strictness}""#);
    let error = initialize_xecguard(&params(&extras), None).unwrap_err();
    assert!(matches!(error, ThirdPartyInitBError::InvalidParam { .. }));
}
