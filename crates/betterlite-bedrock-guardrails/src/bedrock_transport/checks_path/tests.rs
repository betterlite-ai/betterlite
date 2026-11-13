#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_request::{PrepareError, PrepareInput, RequestEnvironment, prepare_request};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_with, signing_time};
use crate::constants::BEDROCK_CHECKS_PATH;

fn prepare(guardrail: &BedrockGuardrail) -> Result<AwsSignedRequest, PrepareError> {
    let static_credentials = aws_credentials();
    prepare_request(
        guardrail,
        PrepareInput {
            environment: RequestEnvironment::default(),
            credentials: Some(&static_credentials),
            region: "us-west-2",
            api_key: None,
            extra_headers: &[],
            request_path: Some(BEDROCK_CHECKS_PATH),
            now: signing_time(),
        },
        &BTreeMap::from([("source", "INPUT")]),
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
