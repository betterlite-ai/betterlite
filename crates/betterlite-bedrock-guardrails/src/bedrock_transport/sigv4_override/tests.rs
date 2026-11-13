#![cfg(test)]

use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_request::{_prepare_request, PrepareError, PrepareInput, RequestEnvironment};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_with, signing_time};

fn prepare_with(
    guardrail: &BedrockGuardrail,
    extra_headers: &[(String, String)],
) -> Result<AwsSignedRequest, PrepareError> {
    let static_credentials = aws_credentials();
    _prepare_request(
        guardrail,
        PrepareInput {
            environment: RequestEnvironment::default(),
            credentials: Some(&static_credentials),
            region: "us-west-2",
            api_key: None,
            extra_headers,
            request_path: Some("/guardrail-checks/invoke"),
            now: signing_time(),
        },
        &serde_json::json!({"source": "INPUT"}),
    )
}

fn header<'text>(request: &'text AwsSignedRequest, name: &str) -> Option<&'text str> {
    request
        .headers
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn sigv4_override_signs_the_request_with_the_credentials() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let signed = prepare_with(&guardrail, &[]).expect("the signed request");
    let authorization = header(&signed, "Authorization").expect("the signature");
    assert!(authorization.starts_with("AWS4-HMAC-SHA256"));
    assert!(header(&signed, "X-Amz-Date").is_some());
}

#[test]
fn sigv4_override_keeps_an_explicit_authorization_header() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let extra = vec![("Authorization".to_owned(), "Bearer override".to_owned())];
    let signed = prepare_with(&guardrail, &extra).expect("the signed request");
    assert_eq!(header(&signed, "Authorization"), Some("Bearer override"));
}

#[test]
fn sigv4_override_keeps_the_other_extra_headers() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let extra = vec![("x-custom".to_owned(), "kept".to_owned())];
    let signed = prepare_with(&guardrail, &extra).expect("the signed request");
    assert_eq!(header(&signed, "x-custom"), Some("kept"));
    assert!(
        header(&signed, "Authorization")
            .expect("the signature")
            .starts_with("AWS4-HMAC-SHA256")
    );
}

#[test]
fn sigv4_override_requires_credentials() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let error = _prepare_request(
        &guardrail,
        PrepareInput {
            environment: RequestEnvironment::default(),
            credentials: None,
            region: "us-west-2",
            api_key: None,
            extra_headers: &[],
            request_path: Some("/guardrail-checks/invoke"),
            now: signing_time(),
        },
        &serde_json::json!({"source": "INPUT"}),
    )
    .expect_err("the missing credentials");
    assert!(matches!(
        error,
        PrepareError::Signing(SigningError::NoCredentials)
    ));
}
