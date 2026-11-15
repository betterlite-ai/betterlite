#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{StreamingTransformMode, initialize_generic_api};
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[test]
fn generic_api_takes_the_defaults_of_an_empty_config() {
    let built = initialize_generic_api(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert!(built.headers.is_none());
    assert!(built.additional_provider_specific_params.0.is_empty());
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailClosed);
    assert!(built.fail_on_error);
    assert_eq!(built.extra_headers, None);
    assert_eq!(built.streaming_end_of_stream_only, None);
    assert_eq!(built.streaming_sampling_rate, None);
    assert_eq!(built.streaming_transform_mode, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn generic_api_copies_the_configured_fields() {
    let built = initialize_generic_api(
        &params(
            r#",
            "api_base": "https://generic.test",
            "api_key": "generic-key",
            "headers": {"x-tenant": "acme"},
            "additional_provider_specific_params": {"policy": "strict", "level": 2},
            "unreachable_fallback": "fail_open",
            "fail_on_error": false,
            "extra_headers": ["x-request-id"],
            "timeout": 15,
            "default_on": true
            "#,
        ),
        Some("generic-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "generic-guard");
    assert_eq!(built.api_base.as_deref(), Some("https://generic.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("generic-key"));
    let headers = built.headers.unwrap();
    assert_eq!(
        headers.get("x-tenant").map(SecretString::expose_secret),
        Some("acme")
    );
    assert_eq!(
        built.additional_provider_specific_params.0.get("policy"),
        Some(&GuardrailValue::Text("strict".to_owned()))
    );
    assert_eq!(
        built.additional_provider_specific_params.0.get("level"),
        Some(&GuardrailValue::SignedInteger(2))
    );
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailOpen);
    assert!(!built.fail_on_error);
    assert_eq!(built.extra_headers, Some(vec!["x-request-id".to_owned()]));
    assert_eq!(built.timeout, Some(15.0_f64));
    assert!(built.default_on);
}

#[test]
fn generic_api_reads_the_streaming_fields_from_optional_params() {
    let built = initialize_generic_api(
        &params(
            r#",
            "optional_params": {
                "streaming_end_of_stream_only": true,
                "streaming_sampling_rate": 4,
                "streaming_transform_mode": "incremental_diff"
            }
            "#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(built.streaming_end_of_stream_only, Some(true));
    assert_eq!(built.streaming_sampling_rate, Some(4));
    assert_eq!(
        built.streaming_transform_mode,
        Some(StreamingTransformMode::IncrementalDiff)
    );
}

#[test]
fn generic_api_reads_the_streaming_fields_from_the_top_level_without_optional_params() {
    let built = initialize_generic_api(
        &params(
            r#",
            "streaming_end_of_stream_only": false,
            "streaming_sampling_rate": 9,
            "streaming_transform_mode": "block_only"
            "#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(built.streaming_end_of_stream_only, Some(false));
    assert_eq!(built.streaming_sampling_rate, Some(9));
    assert_eq!(
        built.streaming_transform_mode,
        Some(StreamingTransformMode::BlockOnly)
    );
}

#[test]
fn generic_api_lets_an_optional_params_value_win_over_the_top_level() {
    let built = initialize_generic_api(
        &params(
            r#",
            "streaming_end_of_stream_only": false,
            "streaming_sampling_rate": 9,
            "streaming_transform_mode": "block_only",
            "optional_params": {
                "streaming_end_of_stream_only": true,
                "streaming_sampling_rate": 2,
                "streaming_transform_mode": "incremental_diff"
            }
            "#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(built.streaming_end_of_stream_only, Some(true));
    assert_eq!(built.streaming_sampling_rate, Some(2));
    assert_eq!(
        built.streaming_transform_mode,
        Some(StreamingTransformMode::IncrementalDiff)
    );
}

#[test]
fn generic_api_falls_back_to_the_top_level_for_a_null_or_missing_optional_entry() {
    let built = initialize_generic_api(
        &params(
            r#",
            "streaming_end_of_stream_only": true,
            "streaming_sampling_rate": 9,
            "streaming_transform_mode": "incremental_diff",
            "optional_params": {
                "streaming_end_of_stream_only": null,
                "streaming_sampling_rate": null
            }
            "#,
        ),
        None,
    )
    .unwrap();
    assert_eq!(built.streaming_end_of_stream_only, Some(true));
    assert_eq!(built.streaming_sampling_rate, Some(9));
    assert_eq!(
        built.streaming_transform_mode,
        Some(StreamingTransformMode::IncrementalDiff)
    );
}

#[test]
fn generic_api_leaves_a_streaming_field_unset_when_neither_place_has_it() {
    let built = initialize_generic_api(
        &params(r#", "optional_params": {"streaming_sampling_rate": null}"#),
        None,
    )
    .unwrap();
    assert_eq!(built.streaming_sampling_rate, None);
    assert_eq!(built.streaming_end_of_stream_only, None);
}

#[test]
fn generic_api_reads_a_null_top_level_streaming_field_as_unset() {
    let built =
        initialize_generic_api(&params(r#", "streaming_transform_mode": null"#), None).unwrap();
    assert_eq!(built.streaming_transform_mode, None);
}

#[test]
fn generic_api_reads_null_headers_as_unset() {
    let built = initialize_generic_api(&params(r#", "headers": null"#), None).unwrap();
    assert!(built.headers.is_none());
}

#[test]
fn generic_api_reads_null_additional_params_as_the_empty_map() {
    let built = initialize_generic_api(
        &params(r#", "additional_provider_specific_params": null"#),
        None,
    )
    .unwrap();
    assert!(built.additional_provider_specific_params.0.is_empty());
}

#[rstest]
#[case::headers(r#", "headers": ["a"]"#, "headers")]
#[case::rate_text(
    r#", "optional_params": {"streaming_sampling_rate": "often"}"#,
    "streaming_sampling_rate"
)]
#[case::rate_negative(r#", "streaming_sampling_rate": -1"#, "streaming_sampling_rate")]
#[case::eos_text(
    r#", "streaming_end_of_stream_only": "yes""#,
    "streaming_end_of_stream_only"
)]
#[case::mode_unknown(r#", "streaming_transform_mode": "other""#, "streaming_transform_mode")]
#[case::optional_params_text(r#", "optional_params": "text""#, "optional_params")]
fn generic_api_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_generic_api(&params(extras), None).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}
