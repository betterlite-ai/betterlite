#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::initialize_typesafe;
use crate::third_party_init_b::error::ThirdPartyInitBError;

fn params(mode: &str, extras: &str) -> LitellmParams {
    let body = format!(r#"{{"guardrail": "typesafe", "mode": {mode}{extras}}}"#);
    decode_litellm_params(body.as_bytes()).unwrap()
}

#[test]
fn typesafe_refuses_a_missing_name() {
    let missing = initialize_typesafe(&params(r#""pre_call""#, ""), None).unwrap_err();
    assert!(matches!(missing, ThirdPartyInitBError::MissingTypesafeName));
    assert_eq!(missing.to_string(), "TypeSafe guardrail name is required");
}

#[test]
fn typesafe_keeps_an_empty_name_because_litellm_only_indexes_it() {
    let built = initialize_typesafe(&params(r#""pre_call""#, ""), Some("")).unwrap();
    assert_eq!(built.name, "");
}

#[test]
fn typesafe_copies_the_config_values() {
    let built = initialize_typesafe(
        &params(
            r#""pre_call""#,
            r#",
            "api_key": "ts-key",
            "api_base": "https://typesafe.test",
            "model": "jev-x",
            "unreachable_fallback": "fail_closed",
            "timeout": 5.5,
            "default_on": true,
            "optional_params": {
                "relevance_threshold": 0.7,
                "min_chars_to_evaluate": 50,
                "max_result_chars_in_state": 900
            }
            "#,
        ),
        Some("ts-check"),
    )
    .unwrap();
    assert_eq!(built.name, "ts-check");
    assert_eq!(
        built
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("ts-key".to_owned())
    );
    assert_eq!(built.api_base.as_deref(), Some("https://typesafe.test"));
    assert_eq!(built.model.as_deref(), Some("jev-x"));
    assert_eq!(built.relevance_threshold, Some(0.7_f64));
    assert_eq!(built.min_chars_to_evaluate, Some(50));
    assert_eq!(built.max_result_chars_in_state, Some(900));
    assert_eq!(built.timeout, Some(5.5_f64));
    assert_eq!(built.unreachable_fallback.as_deref(), Some("fail_closed"));
    assert!(built.default_on);
}

#[test]
fn typesafe_applies_the_defaults() {
    let built = initialize_typesafe(&params(r#""pre_call""#, ""), Some("ts-check")).unwrap();
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.model, None);
    assert_eq!(built.relevance_threshold, None);
    assert_eq!(built.min_chars_to_evaluate, None);
    assert_eq!(built.max_result_chars_in_state, None);
    assert_eq!(built.timeout, None);
    assert_eq!(built.unreachable_fallback, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn typesafe_ignores_a_top_level_tuning_param() {
    let built = initialize_typesafe(
        &params(r#""pre_call""#, r#", "relevance_threshold": 0.7"#),
        Some("ts-check"),
    )
    .unwrap();
    assert_eq!(built.relevance_threshold, None);
}

#[test]
fn typesafe_coerces_a_mode_list_and_a_tagged_mode() {
    let listed = initialize_typesafe(
        &params(r#"["pre_call", "post_call"]"#, ""),
        Some("ts-check"),
    )
    .unwrap();
    assert_eq!(
        listed.event_hook,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
    let tagged = initialize_typesafe(
        &params(
            r#"{"tags": {"team": "pre_call"}, "default": "post_call"}"#,
            "",
        ),
        Some("ts-check"),
    )
    .unwrap();
    let expected = LitellmParamsMode::Tagged(Mode {
        tags: BTreeMap::from([("team".to_owned(), ModeTarget::One("pre_call".to_owned()))]),
        default: Some(ModeTarget::One("post_call".to_owned())),
    });
    assert_eq!(tagged.event_hook, expected);
}

#[rstest]
#[case::single(r#""banana""#)]
#[case::in_a_list(r#"["pre_call", "banana"]"#)]
fn typesafe_refuses_an_unknown_event_hook(#[case] mode: &str) {
    let error = initialize_typesafe(&params(mode, ""), Some("ts-check")).unwrap_err();
    assert!(matches!(
        error,
        ThirdPartyInitBError::InvalidEventHook { .. }
    ));
}

#[rstest]
#[case::relevance_high(r#""relevance_threshold": 1.5"#)]
#[case::relevance_low(r#""relevance_threshold": -0.1"#)]
#[case::min_chars_negative(r#""min_chars_to_evaluate": -1"#)]
#[case::max_result_zero(r#""max_result_chars_in_state": 0"#)]
fn typesafe_refuses_an_out_of_range_optional_param(#[case] entry: &str) {
    let extras = format!(r#", "optional_params": {{{entry}}}"#);
    let error =
        initialize_typesafe(&params(r#""pre_call""#, &extras), Some("ts-check")).unwrap_err();
    assert!(matches!(error, ThirdPartyInitBError::InvalidParam { .. }));
}

#[rstest]
#[case::relevance_low_bound(r#""relevance_threshold": 0.0"#)]
#[case::relevance_high_bound(r#""relevance_threshold": 1.0"#)]
#[case::min_chars_zero(r#""min_chars_to_evaluate": 0"#)]
#[case::max_result_one(r#""max_result_chars_in_state": 1"#)]
fn typesafe_keeps_an_optional_param_on_its_bound(#[case] entry: &str) {
    let extras = format!(r#", "optional_params": {{{entry}}}"#);
    initialize_typesafe(&params(r#""pre_call""#, &extras), Some("ts-check")).unwrap();
}

#[test]
fn typesafe_optional_params_drop_nulls() {
    let built = initialize_typesafe(
        &params(
            r#""pre_call""#,
            r#", "optional_params": {
                "relevance_threshold": null,
                "min_chars_to_evaluate": null,
                "max_result_chars_in_state": null
            }"#,
        ),
        Some("ts-check"),
    )
    .unwrap();
    assert_eq!(built.relevance_threshold, None);
    assert_eq!(built.min_chars_to_evaluate, None);
    assert_eq!(built.max_result_chars_in_state, None);
}

#[rstest]
#[case::explicit_fail_open("fail_open")]
#[case::explicit_fail_closed("fail_closed")]
fn typesafe_unreachable_fallback_is_the_explicit_param(#[case] fallback: &str) {
    let extras = format!(r#", "unreachable_fallback": "{fallback}""#);
    let built = initialize_typesafe(&params(r#""pre_call""#, &extras), Some("ts-check")).unwrap();
    assert_eq!(built.unreachable_fallback.as_deref(), Some(fallback));
}
