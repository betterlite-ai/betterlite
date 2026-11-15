#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::SecretString;

use crate::bedrock_request::{PrepareInput, RequestEnvironment, prepare_request};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_with, signing_time};

fn prepare(
    guardrail: &BedrockGuardrail,
    api_key: Option<&SecretString>,
    environment: RequestEnvironment<'_>,
) -> AwsSignedRequest {
    let static_credentials = aws_credentials();
    prepare_request(
        guardrail,
        PrepareInput {
            environment,
            credentials: Some(&static_credentials),
            region: "us-west-2",
            api_key,
            extra_headers: &[],
            request_path: Some("/guardrail-checks/invoke"),
            now: signing_time(),
        },
        &BTreeMap::from([("source", "INPUT")]),
    )
    .expect("the signed request")
}

fn header<'text>(request: &'text AwsSignedRequest, name: &str) -> Option<&'text str> {
    request
        .headers
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn bearer_request_sets_the_bearer_authorization() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let api_key = SecretString::from("api-token".to_owned());
    let signed = prepare(&guardrail, Some(&api_key), RequestEnvironment::default());
    assert_eq!(header(&signed, "Authorization"), Some("Bearer api-token"));
    assert_eq!(header(&signed, "X-Amz-Date"), None);
    assert_eq!(header(&signed, "X-Amz-Security-Token"), None);
}

#[test]
fn bearer_request_prefers_the_api_key_over_the_configured_token() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let api_key = SecretString::from("api-token".to_owned());
    let configured = SecretString::from("configured-token".to_owned());
    let environment = RequestEnvironment {
        runtime_endpoint: None,
        bearer_token: Some(&configured),
    };
    let signed = prepare(&guardrail, Some(&api_key), environment);
    assert_eq!(header(&signed, "Authorization"), Some("Bearer api-token"));
}

#[test]
fn bearer_request_uses_the_configured_token_without_an_api_key() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let configured = SecretString::from("configured-token".to_owned());
    let environment = RequestEnvironment {
        runtime_endpoint: None,
        bearer_token: Some(&configured),
    };
    let signed = prepare(&guardrail, None, environment);
    assert_eq!(
        header(&signed, "Authorization"),
        Some("Bearer configured-token")
    );
}

#[test]
fn bearer_request_sends_the_json_content_type_and_body() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let api_key = SecretString::from("api-token".to_owned());
    let signed = prepare(&guardrail, Some(&api_key), RequestEnvironment::default());
    assert_eq!(header(&signed, "Content-Type"), Some("application/json"));
    assert_eq!(signed.body, br#"{"source":"INPUT"}"#.to_vec());
    assert_eq!(signed.method, "POST");
}
