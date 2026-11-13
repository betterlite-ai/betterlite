#![cfg(test)]

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_request::{_prepare_request, PrepareError, PrepareInput, RequestEnvironment};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_with, signing_time};
use crate::constants::_BEDROCK_INVOKE_GUARDRAIL_CHECKS_PATH;

fn prepare(guardrail: &BedrockGuardrail) -> Result<AwsSignedRequest, PrepareError> {
    let static_credentials = aws_credentials();
    _prepare_request(
        guardrail,
        PrepareInput {
            environment: RequestEnvironment::default(),
            credentials: Some(&static_credentials),
            region: "us-west-2",
            api_key: None,
            extra_headers: &[],
            request_path: Some(_BEDROCK_INVOKE_GUARDRAIL_CHECKS_PATH),
            now: signing_time(),
        },
        &serde_json::json!({"messages": []}),
    )
}

#[test]
fn checks_path_is_used_as_is() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let signed = prepare(&guardrail).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/guardrail-checks/invoke"
    );
}

#[test]
fn checks_path_never_interpolates_the_guardrail_identity() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let signed = prepare(&guardrail).expect("the signed request");
    assert!(!signed.url.contains("gr-1"));
    assert!(!signed.url.contains("/version/"));
}
