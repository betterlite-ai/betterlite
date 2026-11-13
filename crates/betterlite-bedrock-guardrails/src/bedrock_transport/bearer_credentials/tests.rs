#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::SecretString;

use crate::bedrock_credentials::{RegionEnvironment, load_credentials};
use crate::bedrock_transport::test_support::{RecordingCredentials, guardrail_with};

#[tokio::test]
async fn bearer_credentials_are_none_and_skip_the_aws_source() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let source = RecordingCredentials::failing(CredentialError::MissingCredentials);
    let token = SecretString::from("bearer-token".to_owned());
    let (credentials, region) = load_credentials(
        &guardrail,
        Some(&token),
        RegionEnvironment::default(),
        &source,
    )
    .await
    .expect("the credentials");
    assert!(credentials.is_none());
    assert_eq!(region, "us-west-2");
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn bearer_credentials_keep_the_configured_region() {
    let params = OpaqueParams(BTreeMap::from([(
        "aws_region_name".to_owned(),
        GuardrailValue::Text("eu-west-3".to_owned()),
    )]));
    let guardrail = guardrail_with(params);
    let source = RecordingCredentials::failing(CredentialError::MissingCredentials);
    let token = SecretString::from("bearer-token".to_owned());
    let (credentials, region) = load_credentials(
        &guardrail,
        Some(&token),
        RegionEnvironment::default(),
        &source,
    )
    .await
    .expect("the credentials");
    assert!(credentials.is_none());
    assert_eq!(region, "eu-west-3");
    assert!(source.calls().is_empty());
}
