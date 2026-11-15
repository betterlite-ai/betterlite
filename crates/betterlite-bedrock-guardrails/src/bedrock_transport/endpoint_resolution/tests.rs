#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use crate::bedrock_request::{PrepareError, PrepareInput, RequestEnvironment, prepare_request};
use crate::bedrock_transport::BedrockGuardrail;
use crate::bedrock_transport::test_support::{aws_credentials, guardrail_with, signing_time};

fn prepare(
    guardrail: &BedrockGuardrail,
    environment: RequestEnvironment<'_>,
) -> Result<AwsSignedRequest, PrepareError> {
    let static_credentials = aws_credentials();
    prepare_request(
        guardrail,
        PrepareInput {
            environment,
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

fn params_with_endpoint(endpoint: &str) -> OpaqueParams {
    OpaqueParams(BTreeMap::from([(
        "aws_bedrock_runtime_endpoint".to_owned(),
        GuardrailValue::Text(endpoint.to_owned()),
    )]))
}

#[test]
fn endpoint_resolution_falls_back_to_the_region_endpoint() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let signed = prepare(&guardrail, RequestEnvironment::default()).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/guardrail/gr-1/version/1/apply"
    );
}

#[test]
fn endpoint_resolution_uses_the_configured_runtime_endpoint() {
    let guardrail = guardrail_with(params_with_endpoint("https://vpce.example/prefix"));
    let signed = prepare(&guardrail, RequestEnvironment::default()).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://vpce.example/prefix/guardrail/gr-1/version/1/apply"
    );
}

#[test]
fn endpoint_resolution_uses_the_environment_endpoint() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let environment = RequestEnvironment {
        runtime_endpoint: Some("https://environment.example"),
        bearer_token: None,
    };
    let signed = prepare(&guardrail, environment).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://environment.example/guardrail/gr-1/version/1/apply"
    );
}

#[test]
fn endpoint_resolution_prefers_the_configured_endpoint() {
    let guardrail = guardrail_with(params_with_endpoint("https://configured.example"));
    let environment = RequestEnvironment {
        runtime_endpoint: Some("https://environment.example"),
        bearer_token: None,
    };
    let signed = prepare(&guardrail, environment).expect("the signed request");
    assert_eq!(
        signed.url,
        "https://configured.example/guardrail/gr-1/version/1/apply"
    );
}

#[test]
fn endpoint_resolution_rejects_a_non_string_endpoint() {
    let params = OpaqueParams(BTreeMap::from([(
        "aws_bedrock_runtime_endpoint".to_owned(),
        GuardrailValue::Boolean(true),
    )]));
    let guardrail = guardrail_with(params);
    let error = prepare(&guardrail, RequestEnvironment::default()).expect_err("the endpoint type");
    assert!(matches!(
        error,
        PrepareError::Field {
            field: "aws_bedrock_runtime_endpoint"
        }
    ));
}
