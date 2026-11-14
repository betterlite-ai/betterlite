#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::SecretString;

use crate::bedrock_credentials::{CredentialLoadError, RegionEnvironment, load_credentials};
use crate::bedrock_transport::test_support::{RecordingCredentials, guardrail_with};

fn params_with_region(region: &str) -> OpaqueParams {
    OpaqueParams(BTreeMap::from([(
        "aws_region_name".to_owned(),
        GuardrailValue::Text(region.to_owned()),
    )]))
}

async fn region_of(params: OpaqueParams, environment: RegionEnvironment<'_>) -> String {
    let guardrail = guardrail_with(params);
    let source = RecordingCredentials::failing(CredentialError::MissingCredentials);
    let token = SecretString::from("token".to_owned());
    let (_credentials, region) = load_credentials(&guardrail, Some(&token), environment, &source)
        .await
        .expect("the region");
    region
}

#[tokio::test]
async fn region_resolution_uses_the_explicit_setting() {
    let params = params_with_region("eu-central-1");
    assert_eq!(
        region_of(params, RegionEnvironment::default()).await,
        "eu-central-1"
    );
}

#[tokio::test]
async fn region_resolution_reads_aws_region_name() {
    let environment = RegionEnvironment {
        region_name: Some("ap-south-1"),
        region: None,
    };
    assert_eq!(
        region_of(OpaqueParams::default(), environment).await,
        "ap-south-1"
    );
}

#[tokio::test]
async fn region_resolution_reads_aws_region() {
    let environment = RegionEnvironment {
        region_name: None,
        region: Some("us-east-2"),
    };
    assert_eq!(
        region_of(OpaqueParams::default(), environment).await,
        "us-east-2"
    );
}

#[tokio::test]
async fn region_resolution_prefers_aws_region_over_aws_region_name() {
    let environment = RegionEnvironment {
        region_name: Some("ap-south-1"),
        region: Some("us-east-2"),
    };
    assert_eq!(
        region_of(OpaqueParams::default(), environment).await,
        "us-east-2"
    );
}

#[tokio::test]
async fn region_resolution_defaults_to_us_west_2() {
    assert_eq!(
        region_of(OpaqueParams::default(), RegionEnvironment::default()).await,
        "us-west-2"
    );
}

#[tokio::test]
async fn region_resolution_rejects_an_invalid_region() {
    let guardrail = guardrail_with(params_with_region("EU_WEST"));
    let source = RecordingCredentials::failing(CredentialError::MissingCredentials);
    let error = load_credentials(&guardrail, None, RegionEnvironment::default(), &source)
        .await
        .expect_err("the invalid region");
    assert!(matches!(error, CredentialLoadError::Endpoint(_)));
}
