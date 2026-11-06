#![cfg(test)]

use std::error::Error as _;

use rstest::rstest;

use super::fixture::{Reply, blank_settings, built, explicit_settings, mcp_request, secret};
use crate::mcp_sigv4::error::McpSigV4Error;
use crate::mcp_sigv4::{McpSigV4Auth, McpSigV4Settings};

const ROLE: &str = "arn:aws:iam::123456789012:role/mcp";

fn role_settings() -> McpSigV4Settings {
    McpSigV4Settings {
        aws_role_name: Some(ROLE.to_owned()),
        ..blank_settings()
    }
}

const ENVIRONMENT_CREDENTIALS: [(&str, &str); 3] = [
    ("AWS_ACCESS_KEY_ID", "AKIDFROMENV"),
    ("AWS_SECRET_ACCESS_KEY", "envsecret"),
    ("AWS_EC2_METADATA_DISABLED", "true"),
];

#[tokio::test]
async fn sts_assume_role_signs_with_the_temporary_credentials() {
    let (auth, recorder, _clock) =
        built(role_settings(), &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    let signed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    let authorization = signed.headers()["authorization"].to_str().unwrap();
    assert!(
        authorization
            .contains("Credential=ASIATEMP/20150830/us-east-1/bedrock-agentcore/aws4_request")
    );
    assert_eq!(signed.headers()["x-amz-security-token"], "temptoken");
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].body.contains("Action=AssumeRole"));
    assert!(
        calls[0]
            .body
            .contains("RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2Fmcp")
    );
}

#[tokio::test]
async fn sts_assume_role_authenticates_the_call_with_explicit_creds() {
    let mut settings = explicit_settings();
    settings.aws_role_name = Some(ROLE.to_owned());
    settings.aws_session_token = Some(secret("caller-token"));
    let (auth, recorder, _clock) = built(settings, &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .authorization
            .contains("Credential=AKIDEXAMPLE/20150830/us-east-1/sts/aws4_request")
    );
    assert_eq!(calls[0].security_token, "caller-token");
    let signed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    let authorization = signed.headers()["authorization"].to_str().unwrap();
    assert!(authorization.contains("Credential=ASIATEMP/"));
    assert!(!authorization.contains("AKIDEXAMPLE"));
}

#[tokio::test]
async fn sts_assume_role_uses_the_default_chain_for_the_call_without_explicit_creds() {
    let (_auth, recorder, _clock) =
        built(role_settings(), &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    assert!(
        recorder.calls()[0]
            .authorization
            .contains("Credential=AKIDFROMENV/")
    );
}

#[rstest]
#[case::default_region(None, "https://sts.us-east-1.amazonaws.com/")]
#[case::configured_region(Some("eu-west-1"), "https://sts.eu-west-1.amazonaws.com/")]
#[tokio::test]
async fn sts_assume_role_calls_the_regional_endpoint(
    #[case] region: Option<&str>,
    #[case] endpoint: &str,
) {
    let mut settings = role_settings();
    settings.aws_region_name = region.map(str::to_owned);
    let (_auth, recorder, _clock) = built(settings, &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    assert_eq!(recorder.calls()[0].uri, endpoint);
}

#[tokio::test]
async fn sts_session_name_defaults_to_litellm_mcp_epoch_seconds() {
    let (_auth, recorder, _clock) =
        built(role_settings(), &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    assert!(
        recorder.calls()[0]
            .body
            .contains("RoleSessionName=litellm-mcp-1440938160")
    );
}

#[tokio::test]
async fn sts_session_name_uses_the_configured_name() {
    let mut settings = role_settings();
    settings.aws_session_name = Some("mine".to_owned());
    let (_auth, recorder, _clock) = built(settings, &ENVIRONMENT_CREDENTIALS, Reply::Sts).await;
    assert!(recorder.calls()[0].body.contains("RoleSessionName=mine"));
}

#[tokio::test]
async fn sts_assume_role_failure_propagates_with_its_source() {
    let denied = Reply::Status(
        403,
        "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>no</Message></Error></ErrorResponse>",
    );
    let clock = super::fixture::Clock::at(super::fixture::VECTOR_TIME);
    let (sources, _recorder) =
        super::fixture::sources(&ENVIRONMENT_CREDENTIALS, &[], &clock, denied);
    let error = McpSigV4Auth::new(role_settings(), sources)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, McpSigV4Error::AssumeRole { .. }));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn sts_without_any_credentials_for_the_call_fails_as_assume_role() {
    let environment = [("AWS_EC2_METADATA_DISABLED", "true")];
    let clock = super::fixture::Clock::at(super::fixture::VECTOR_TIME);
    let (sources, _recorder) = super::fixture::sources(&environment, &[], &clock, Reply::Sts);
    let error = McpSigV4Auth::new(role_settings(), sources)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, McpSigV4Error::AssumeRole { .. }));
}
