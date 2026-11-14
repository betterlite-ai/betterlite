#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::ExposeSecret;

use crate::bedrock_credentials::{CredentialLoadError, RegionEnvironment, load_credentials};
use crate::bedrock_transport::test_support::{
    RecordingCredentials, aws_credentials, guardrail_with,
};

fn aws_parameters() -> OpaqueParams {
    let pairs = [
        ("aws_access_key_id", "AKIAEXAMPLE"),
        ("aws_secret_access_key", "secret"),
        ("aws_session_token", "session"),
        ("aws_region_name", "eu-west-1"),
        ("aws_role_name", "role"),
        ("aws_session_name", "session-name"),
        ("aws_profile_name", "profile"),
        ("aws_web_identity_token", "web-identity"),
        ("aws_sts_endpoint", "https://sts.example"),
        ("aws_external_id", "external"),
    ];
    OpaqueParams(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), GuardrailValue::Text(value.to_owned())))
            .collect(),
    )
}

fn first_call(source: &RecordingCredentials) -> (AwsSettings, String) {
    let mut calls = source.calls();
    assert_eq!(calls.len(), 1);
    calls.remove(0)
}

#[tokio::test]
async fn credential_fetch_passes_every_aws_setting() {
    let guardrail = guardrail_with(aws_parameters());
    let source = RecordingCredentials::replying(aws_credentials());
    let (loaded, region) =
        load_credentials(&guardrail, None, RegionEnvironment::default(), &source)
            .await
            .expect("the credentials");
    let resolved = loaded.expect("the resolved credentials");
    assert_eq!(
        resolved.access_key_id.expose_secret(),
        "AKIAIOSFODNN7EXAMPLE"
    );
    assert_eq!(region, "eu-west-1");
    let (settings, called_region) = first_call(&source);
    assert_eq!(called_region, "eu-west-1");
    assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(
        settings
            .access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("AKIAEXAMPLE")
    );
    assert_eq!(
        settings
            .secret_access_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("secret")
    );
    assert_eq!(
        settings
            .session_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("session")
    );
    assert_eq!(
        settings
            .web_identity_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("web-identity")
    );
    assert_eq!(settings.session_name.as_deref(), Some("session-name"));
    assert_eq!(settings.profile_name.as_deref(), Some("profile"));
    assert_eq!(settings.role_name.as_deref(), Some("role"));
    assert_eq!(
        settings.sts_endpoint.as_deref(),
        Some("https://sts.example")
    );
    assert_eq!(settings.external_id.as_deref(), Some("external"));
    assert!(settings.bearer_token.is_none());
}

#[tokio::test]
async fn credential_fetch_uses_the_environment_region() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let source = RecordingCredentials::replying(aws_credentials());
    let environment = RegionEnvironment {
        region_name: None,
        region: Some("us-east-2"),
    };
    let (loaded, region) = load_credentials(&guardrail, None, environment, &source)
        .await
        .expect("the credentials");
    let resolved = loaded.expect("the resolved credentials");
    assert_eq!(
        resolved.access_key_id.expose_secret(),
        "AKIAIOSFODNN7EXAMPLE"
    );
    assert_eq!(region, "us-east-2");
    let (settings, called_region) = first_call(&source);
    assert_eq!(called_region, "us-east-2");
    assert_eq!(settings.region_name.as_deref(), Some("us-east-2"));
}

#[tokio::test]
async fn credential_fetch_propagates_a_resolution_failure() {
    let guardrail = guardrail_with(OpaqueParams::default());
    let source = RecordingCredentials::failing(CredentialError::MissingCredentials);
    let error = load_credentials(&guardrail, None, RegionEnvironment::default(), &source)
        .await
        .expect_err("the credential error");
    assert!(matches!(error, CredentialLoadError::Credential(_)));
}

#[tokio::test]
async fn credential_fetch_rejects_a_non_string_region() {
    let params = OpaqueParams(BTreeMap::from([(
        "aws_region_name".to_owned(),
        GuardrailValue::Boolean(true),
    )]));
    let guardrail = guardrail_with(params);
    let source = RecordingCredentials::replying(aws_credentials());
    let error = load_credentials(&guardrail, None, RegionEnvironment::default(), &source)
        .await
        .expect_err("the region type error");
    assert!(matches!(
        error,
        CredentialLoadError::Field {
            field: "aws_region_name"
        }
    ));
    assert!(source.calls().is_empty());
}
