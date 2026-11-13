use rstest::rstest;

use crate::health_probe_settings::display::{
    ADMIN_ONLY_DISPLAY_PARAMS, HEALTH_DISPLAY_PARAMS, MINIMAL_DISPLAY_PARAMS, clean_endpoint_data,
};
use crate::health_probe_settings::tests::support::object;

#[test]
fn the_display_field_sets_match_litellm() {
    let mut health = HEALTH_DISPLAY_PARAMS.to_vec();
    health.sort_unstable();
    let mut expected = vec![
        "api_base",
        "api_version",
        "aws_bedrock_runtime_endpoint",
        "aws_region_name",
        "base_model",
        "custom_llm_provider",
        "error",
        "mode",
        "mode_error",
        "model",
        "raw_request_typed_dict",
        "region_name",
        "rpm",
        "tpm",
        "vertex_location",
        "vertex_project",
        "watsonx_region_name",
        "x-ms-region",
        "x-ratelimit-remaining-requests",
        "x-ratelimit-remaining-tokens",
    ];
    expected.sort_unstable();
    assert_eq!(health, expected);
    assert_eq!(MINIMAL_DISPLAY_PARAMS, ["model", "mode_error"]);
    assert_eq!(
        ADMIN_ONLY_DISPLAY_PARAMS,
        ["api_base", "api_version", "aws_bedrock_runtime_endpoint"]
    );
}

#[test]
fn details_keep_the_approved_fields_and_drop_the_rest() {
    let data = object(
        r#"{"model":"m","api_base":"https://x","mode_error":"none","api_key":"secret","x-ms-region":"east"}"#,
    );
    let cleaned = clean_endpoint_data(&data, true);
    assert_eq!(
        cleaned,
        object(r#"{"model":"m","api_base":"https://x","mode_error":"none","x-ms-region":"east"}"#)
    );
}

#[test]
fn without_details_only_the_minimal_fields_survive() {
    let data = object(r#"{"model":"m","mode_error":"none","api_base":"https://x","mode":"chat"}"#);
    let cleaned = clean_endpoint_data(&data, false);
    assert_eq!(cleaned, object(r#"{"model":"m","mode_error":"none"}"#));
}

#[rstest]
#[case::a_near_miss("mode_error_extra")]
#[case::a_prefixed_name("api_base_url")]
#[case::an_unlisted_field("litellm_params")]
fn a_field_outside_the_set_is_dropped(#[case] field: &str) {
    let data = object(&format!(r#"{{"{field}":true,"model":"m"}}"#));
    assert_eq!(clean_endpoint_data(&data, true), object(r#"{"model":"m"}"#));
}

#[test]
fn the_approved_values_are_kept_as_given() {
    let data = object(r#"{"error":{"message":"down","code":500},"mode":"chat"}"#);
    assert_eq!(
        clean_endpoint_data(&data, true),
        object(r#"{"error":{"message":"down","code":500},"mode":"chat"}"#)
    );
}
