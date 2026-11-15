#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::signing::request::AwsSignedRequest;

use crate::bedrock_request::{PrepareError, PrepareInput, RequestEnvironment, prepare_request};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_init, signing_time};

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
            request_path: None,
            now: signing_time(),
        },
        &BTreeMap::from([("source", "INPUT")]),
    )
}

#[test]
fn apply_guardrail_path_interpolates_the_identity_and_version() {
    let guardrail = BedrockGuardrail::__init__(guardrail_init()).expect("the guardrail");
    let signed = prepare(&guardrail).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/guardrail/gr-1/version/1/apply"
    );
}

#[test]
fn apply_guardrail_path_requires_the_identifier() {
    let mut init = guardrail_init();
    init.guardrail_identifier = None;
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    let error = prepare(&guardrail).expect_err("the missing identifier");
    assert!(matches!(
        error,
        PrepareError::MissingIdentity {
            field: "guardrailIdentifier"
        }
    ));
}

#[test]
fn apply_guardrail_path_requires_the_version() {
    let mut init = guardrail_init();
    init.guardrail_version = None;
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    let error = prepare(&guardrail).expect_err("the missing version");
    assert!(matches!(
        error,
        PrepareError::MissingIdentity {
            field: "guardrailVersion"
        }
    ));
}
