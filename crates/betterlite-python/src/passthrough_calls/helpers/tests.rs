#![cfg(test)]

use pyo3::prelude::*;

use super::{
    encode_arn, forward_headers_from_request, merge_query_parameters, resolve_passthrough_timeout,
    resolve_sdk_timeout,
};

fn assert_same_seconds(actual: f64, expected: f64) {
    assert_eq!(actual.to_bits(), expected.to_bits());
}

#[test]
fn merge_query_parameters_layers_defaults_below_the_url_and_the_request() {
    let merged = merge_query_parameters(
        "https://example.test/v1?from_url=url&shared=url&blank=",
        r#"{"shared": "request", "tags": ["a", "b"]}"#,
        Some(r#"{"shared": "default", "from_default": "default", "from_url": "default"}"#),
    )
    .unwrap();
    assert_eq!(
        merged,
        r#"{"shared":"request","from_default":"default","from_url":"url","tags":["a","b"]}"#
    );
}

#[test]
fn merge_query_parameters_reads_a_repeated_url_parameter_as_a_list() {
    let merged = merge_query_parameters("https://example.test/v1?tag=x&tag=y", "{}", None).unwrap();
    assert_eq!(merged, r#"{"tag":["x","y"]}"#);
}

#[test]
fn merge_query_parameters_reads_the_query_of_a_scheme_less_url() {
    let merged = merge_query_parameters("/v1/chat?from_url=url&shared=url", "{}", None).unwrap();
    assert_eq!(merged, r#"{"from_url":"url","shared":"url"}"#);
}

#[test]
fn merge_query_parameters_refuses_a_url_that_is_not_one() {
    Python::initialize();
    let error = merge_query_parameters("http://", "{}", None).unwrap_err();
    assert!(error.to_string().contains("not a URL"), "{error}");
}

#[test]
fn forward_headers_from_request_merges_the_request_headers_when_asked() {
    let merged = forward_headers_from_request(
        concat!(
            r#"{"host": "upstream.test", "content-length": "5", "accept-encoding": "br", "#,
            r#""x-request": "1", "X-Custom": "old"}"#
        ),
        r#"{"x-custom": "new"}"#,
        true,
    )
    .unwrap();
    assert_eq!(merged, r#"{"x-request":"1","x-custom":"new"}"#);
}

#[test]
fn forward_headers_from_request_keeps_only_the_given_headers_when_not_asked() {
    let merged = forward_headers_from_request(
        r#"{"host": "upstream.test", "x-request": "1"}"#,
        r#"{"authorization": "Bearer sk-test"}"#,
        false,
    )
    .unwrap();
    assert_eq!(merged, r#"{"authorization":"Bearer sk-test"}"#);
}

#[test]
fn forward_headers_from_request_strips_the_pass_prefix_and_skips_protected_names() {
    let merged = forward_headers_from_request(
        concat!(
            r#"{"x-pass-anthropic-beta": "beta-1", "x-pass-Authorization": "secret", "#,
            r#""x-pass-x-amz-date": "2026", "x-request": "1"}"#
        ),
        "{}",
        false,
    )
    .unwrap();
    assert_eq!(merged, r#"{"anthropic-beta":"beta-1"}"#);
}

#[test]
fn encode_bedrock_runtime_modelid_arn_encodes_an_application_inference_profile() {
    let endpoint =
        "/model/arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc123/invoke";
    assert_eq!(
        encode_arn(endpoint),
        "/model/arn:aws:bedrock:us-east-1:123456789012:application-inference-profile%2Fabc123/invoke"
    );
}

#[test]
fn encode_bedrock_runtime_modelid_arn_leaves_an_endpoint_without_an_arn_alone() {
    let endpoint = "/model/anthropic.claude-3/invoke";
    assert_eq!(encode_arn(endpoint), endpoint);
}

#[test]
fn resolve_passthrough_timeout_prefers_the_endpoint_value() {
    assert_same_seconds(resolve_passthrough_timeout(Some(12.5_f64)), 12.5_f64);
    assert_same_seconds(resolve_passthrough_timeout(Some(0.0_f64)), 0.0_f64);
}

#[test]
fn resolve_passthrough_timeout_falls_back_to_the_default() {
    assert_same_seconds(resolve_passthrough_timeout(None), 600.0_f64);
}

#[test]
fn resolve_sdk_timeout_falls_back_to_the_default() {
    assert_same_seconds(resolve_sdk_timeout("{}").unwrap(), 600.0_f64);
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"timeout": null}}"#).unwrap(),
        600.0_f64,
    );
}

#[test]
fn resolve_sdk_timeout_prefers_the_request_timeout_fields() {
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"timeout": 12}}"#).unwrap(),
        12.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"request_timeout": "30"}}"#).unwrap(),
        30.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(concat!(
            r#"{"kwargs": {"timeout": null, "request_timeout": "30"}, "#,
            r#""litellm_params": {"timeout": 40}}"#
        ))
        .unwrap(),
        30.0_f64,
    );
}

#[test]
fn resolve_sdk_timeout_uses_the_deployment_and_router_fields() {
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"litellm_params": {"timeout": 20}, "router_timeout": 40}"#)
            .unwrap(),
        20.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"litellm_params": {"request_timeout": 25}, "router_timeout": 40}"#)
            .unwrap(),
        25.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"router_timeout": 40}"#).unwrap(),
        40.0_f64,
    );
}

#[test]
fn resolve_sdk_timeout_puts_the_stream_levels_first() {
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"stream": true, "stream_timeout": 5, "timeout": 9}}"#)
            .unwrap(),
        5.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(concat!(
            r#"{"kwargs": {"stream": true}, "litellm_params": {"stream_timeout": 6}, "#,
            r#""router_stream_timeout": 7}"#
        ))
        .unwrap(),
        6.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"stream": true}, "router_stream_timeout": 7}"#).unwrap(),
        7.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(
            r#"{"kwargs": {"stream": true, "timeout": 9}, "router_stream_timeout": 7}"#,
        )
        .unwrap(),
        7.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"stream": false, "stream_timeout": 5, "timeout": 9}}"#)
            .unwrap(),
        9.0_f64,
    );
}

#[test]
fn resolve_sdk_timeout_reads_a_bool_as_seconds() {
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"timeout": true}}"#).unwrap(),
        1.0_f64,
    );
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"timeout": false}}"#).unwrap(),
        0.0_f64,
    );
}

#[test]
fn resolve_sdk_timeout_names_the_field_of_a_bad_value() {
    Python::initialize();
    let error = resolve_sdk_timeout(r#"{"kwargs": {"timeout": "soon"}}"#).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("kwargs.timeout"), "{message}");
    assert!(message.contains("soon"), "{message}");
}

#[test]
fn resolve_sdk_timeout_ignores_a_bad_value_in_a_lower_field() {
    assert_same_seconds(
        resolve_sdk_timeout(r#"{"kwargs": {"timeout": 3}, "litellm_params": {"timeout": "soon"}}"#)
            .unwrap(),
        3.0_f64,
    );
}
