#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::{
    ChunkBudgetChars, SamplingRate,
};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{BedrockArgs, initialize_bedrock};
use crate::guardrail_initializers::error::InitializerError;

const CONFIGURED: &str = r#",
    "guardrailIdentifier": "id-1",
    "guardrailVersion": "DRAFT",
    "content_filter_threshold": 0.25,
    "prompt_attack_threshold": 0.75,
    "pii_confidence_threshold": 0.9,
    "chunk_budget_chars": 100,
    "contextual_grounding_from_messages": true,
    "disable_exception_on_block": true,
    "default_on": true,
    "mask_request_content": true,
    "mask_response_content": true,
    "experimental_use_latest_role_message_only": true,
    "only_scan_new_messages": true,
    "timeout": 2.5,
    "aws_region_name": "us-east-1",
    "aws_access_key_id": "AKIA",
    "aws_secret_access_key": "secret",
    "aws_session_token": "token",
    "aws_session_name": "sess",
    "aws_profile_name": "profile",
    "aws_role_name": "role",
    "aws_web_identity_token": "web",
    "aws_sts_endpoint": "https://sts",
    "aws_external_id": "ext",
    "aws_bedrock_runtime_endpoint": "https://bedrock"
    "#;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn args_with(extras: &str) -> Result<BedrockArgs, InitializerError> {
    let body = format!(r#"{{"guardrail": "bedrock", "mode": "pre_call"{extras}}}"#);
    initialize_bedrock(&params(&body), Some("bed-check"))
}

fn args(extras: &str) -> BedrockArgs {
    args_with(extras).unwrap()
}

#[test]
fn streaming_params_take_litellm_defaults() {
    let built = args("");
    assert!(built.streaming.buffer_until_moderated);
    assert_eq!(built.streaming.streaming_sampling_rate, SamplingRate(5));
    assert!(!built.streaming.streaming_end_of_stream_only);
    assert!(!built.streaming.buffer_release_on_scan);
}

#[test]
fn streaming_params_skip_null_and_unknown_extras() {
    let built = args(
        r#",
        "streaming_buffer_until_moderated": null,
        "streaming_sampling_rate": null,
        "streaming_end_of_stream_only": null,
        "streaming_buffer_release_on_scan": null,
        "streaming_unrelated": "ignored"
        "#,
    );
    assert!(built.streaming.buffer_until_moderated);
    assert_eq!(built.streaming.streaming_sampling_rate, SamplingRate(5));
    assert!(!built.streaming.streaming_end_of_stream_only);
    assert!(!built.streaming.buffer_release_on_scan);
}

#[test]
fn streaming_params_copy_each_configured_value() {
    let built = args(
        r#",
        "streaming_buffer_until_moderated": false,
        "streaming_sampling_rate": 1,
        "streaming_end_of_stream_only": true,
        "streaming_buffer_release_on_scan": true
        "#,
    );
    assert!(!built.streaming.buffer_until_moderated);
    assert_eq!(built.streaming.streaming_sampling_rate, SamplingRate(1));
    assert!(built.streaming.streaming_end_of_stream_only);
    assert!(built.streaming.buffer_release_on_scan);
}

#[test]
fn streaming_sampling_rate_of_one_is_kept() {
    let built = args(r#", "streaming_sampling_rate": 1"#);
    assert_eq!(built.streaming.streaming_sampling_rate, SamplingRate(1));
}

#[test]
fn streaming_sampling_rate_of_zero_is_refused() {
    let error = args_with(r#", "streaming_sampling_rate": 0"#).unwrap_err();
    assert!(matches!(
        error,
        InitializerError::InvalidStreamingParam { name } if name == "streaming_sampling_rate"
    ));
}

#[rstest]
#[case::text_rate(r#", "streaming_sampling_rate": "5""#, "streaming_sampling_rate")]
#[case::negative_rate(r#", "streaming_sampling_rate": -1"#, "streaming_sampling_rate")]
#[case::fractional_rate(r#", "streaming_sampling_rate": 2.5"#, "streaming_sampling_rate")]
#[case::text_buffer(
    r#", "streaming_buffer_until_moderated": "yes""#,
    "streaming_buffer_until_moderated"
)]
#[case::number_release(
    r#", "streaming_buffer_release_on_scan": 1"#,
    "streaming_buffer_release_on_scan"
)]
fn streaming_wrong_type_names_the_field(#[case] extras: &str, #[case] field: &str) {
    let error = args_with(extras).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidStreamingParam { name } if name.as_str() == field),
        "unexpected error {error}"
    );
}

#[test]
fn checks_are_kept_only_in_the_bedrock_form() {
    let bedrock_checks =
        args(r#", "checks": {"contentFilter": {"categories": [{"category": "VIOLENCE"}]}}"#);
    assert!(bedrock_checks.config.checks.is_some());
    let decision = initialize_bedrock(
        &params(
            r#"{"guardrail": "decision_model", "mode": "pre_call", "checks": [{"name": "n", "instructions": "i"}]}"#,
        ),
        Some("decision"),
    )
    .unwrap();
    assert!(decision.config.checks.is_none());
    assert!(args("").config.checks.is_none());
}

#[test]
fn only_scan_new_messages_defaults_to_false_and_copies_true() {
    assert!(!args("").only_scan_new_messages);
    assert!(args(r#", "only_scan_new_messages": true"#).only_scan_new_messages);
    assert!(
        args(r#", "experimental_use_latest_role_message_only": true"#).latest_role_message_only
    );
}

#[test]
fn bedrock_args_take_the_base_defaults() {
    let built = args("");
    assert_eq!(built.name, "bed-check");
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert!(!built.default_on);
    assert!(!built.mask_request_content);
    assert!(!built.mask_response_content);
    assert!(!built.latest_role_message_only);
    assert_eq!(built.timeout, None);
}

#[test]
fn bedrock_config_takes_the_declared_defaults() {
    let built = args("");
    assert!(built.config.guardrail_identifier.is_none());
    assert!(built.config.guardrail_version.is_none());
    assert_eq!(
        built.config.content_filter_threshold.map(|value| value.0),
        Some(0.5_f64)
    );
    assert_eq!(
        built.config.prompt_attack_threshold.map(|value| value.0),
        Some(0.5_f64)
    );
    assert_eq!(
        built.config.pii_confidence_threshold.map(|value| value.0),
        Some(0.5_f64)
    );
    assert_eq!(built.config.chunk_budget_chars, ChunkBudgetChars(25_000));
    assert!(!built.config.contextual_grounding);
    assert_eq!(built.config.disable_exception_on_block, Some(false));
}

#[test]
fn bedrock_config_leaves_the_aws_credentials_unset() {
    let built = args("");
    assert!(built.config.aws_region_name.is_none());
    assert!(built.config.aws_secret_access_key.is_none());
    assert!(built.config.aws_bedrock_runtime_endpoint.is_none());
}

#[test]
fn bedrock_args_copy_the_configured_base_fields() {
    let built = args(CONFIGURED);
    assert!(built.default_on);
    assert!(built.mask_request_content);
    assert!(built.mask_response_content);
    assert!(built.latest_role_message_only);
    assert!(built.only_scan_new_messages);
    assert_eq!(built.timeout, Some(2.5_f64));
}

#[test]
fn bedrock_config_copies_the_identifiers_and_thresholds() {
    let built = args(CONFIGURED);
    assert_eq!(built.config.guardrail_identifier.as_deref(), Some("id-1"));
    assert_eq!(built.config.guardrail_version.as_deref(), Some("DRAFT"));
    assert_eq!(
        built.config.content_filter_threshold.map(|value| value.0),
        Some(0.25_f64)
    );
    assert_eq!(
        built.config.prompt_attack_threshold.map(|value| value.0),
        Some(0.75_f64)
    );
    assert_eq!(
        built.config.pii_confidence_threshold.map(|value| value.0),
        Some(0.9_f64)
    );
    assert_eq!(built.config.chunk_budget_chars, ChunkBudgetChars(100));
    assert!(built.config.contextual_grounding);
    assert_eq!(built.config.disable_exception_on_block, Some(true));
}

#[test]
fn bedrock_config_copies_the_aws_region_keys_and_session() {
    let built = args(CONFIGURED);
    assert_eq!(built.config.aws_region_name.as_deref(), Some("us-east-1"));
    assert_eq!(built.config.aws_access_key_id.as_deref(), Some("AKIA"));
    assert_eq!(
        built
            .config
            .aws_secret_access_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "secret"
    );
    assert_eq!(
        built
            .config
            .aws_session_token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "token"
    );
    assert_eq!(built.config.aws_session_name.as_deref(), Some("sess"));
    assert_eq!(built.config.aws_profile_name.as_deref(), Some("profile"));
}

#[test]
fn bedrock_config_copies_the_aws_role_and_endpoints() {
    let built = args(CONFIGURED);
    assert_eq!(built.config.aws_role_name.as_deref(), Some("role"));
    assert_eq!(
        built
            .config
            .aws_web_identity_token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "web"
    );
    assert_eq!(
        built.config.aws_sts_endpoint.as_deref(),
        Some("https://sts")
    );
    assert_eq!(built.config.aws_external_id.as_deref(), Some("ext"));
    assert_eq!(
        built.config.aws_bedrock_runtime_endpoint.as_deref(),
        Some("https://bedrock")
    );
}

#[test]
fn bedrock_name_defaults_to_the_empty_string() {
    let built = initialize_bedrock(
        &params(r#"{"guardrail": "bedrock", "mode": "pre_call"}"#),
        None,
    )
    .unwrap();
    assert_eq!(built.name, "");
}

#[test]
fn thresholds_accept_null() {
    let detect_only = args(
        r#",
        "content_filter_threshold": null,
        "prompt_attack_threshold": null,
        "pii_confidence_threshold": null
        "#,
    );
    assert_eq!(detect_only.config.content_filter_threshold, None);
    assert_eq!(detect_only.config.prompt_attack_threshold, None);
    assert_eq!(detect_only.config.pii_confidence_threshold, None);
}

#[rstest]
#[case::above_one("1.5")]
#[case::below_zero("-0.1")]
fn thresholds_refuse_out_of_range_values(#[case] value: &str) {
    let error = args_with(&format!(r#", "content_filter_threshold": {value}"#)).unwrap_err();
    assert!(matches!(
        error,
        InitializerError::InvalidParam(name) if name == "content_filter_threshold"
    ));
}

#[rstest]
#[case::zero(r#", "chunk_budget_chars": 0"#, "chunk_budget_chars")]
#[case::negative(r#", "chunk_budget_chars": -4"#, "chunk_budget_chars")]
#[case::text(r#", "chunk_budget_chars": "many""#, "chunk_budget_chars")]
#[case::fraction(r#", "chunk_budget_chars": 2.5"#, "chunk_budget_chars")]
#[case::text_region(r#", "aws_region_name": 5"#, "aws_region_name")]
#[case::text_threshold(r#", "prompt_attack_threshold": "high""#, "prompt_attack_threshold")]
fn bad_bedrock_extras_name_the_field(#[case] extras: &str, #[case] field: &str) {
    let error = args_with(extras).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidParam(name) if name.as_str() == field),
        "unexpected error {error}"
    );
}
