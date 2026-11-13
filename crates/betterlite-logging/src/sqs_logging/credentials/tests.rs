#![cfg(test)]

use std::sync::Arc;

use secrecy::SecretString;

use crate::sqs_logging::tests::support::environment;

use super::{CredentialError, CredentialParams, CredentialSource as _, EnvironmentCredentials};

fn credentials(variables: &[(&str, &str)]) -> EnvironmentCredentials {
    EnvironmentCredentials {
        environment: Arc::new(environment(variables)),
    }
}

fn params() -> CredentialParams {
    CredentialParams::default()
}

fn explicit() -> CredentialParams {
    let mut supplied = params();
    supplied.access_key_id = Some(SecretString::from("access"));
    supplied.secret_access_key = Some(SecretString::from("secret"));
    supplied
}

#[tokio::test]
async fn static_keys_become_the_signing_identity() {
    let resolved = credentials(&[])
        .resolve(&explicit())
        .await
        .expect("the credentials resolve");
    assert_eq!(resolved.access_key_id(), "access");
    assert_eq!(resolved.secret_access_key(), "secret");
    assert!(resolved.session_token().is_none());
}

#[tokio::test]
async fn session_token_is_carried() {
    let mut supplied = explicit();
    supplied.session_token = Some(SecretString::from("token"));
    let resolved = credentials(&[])
        .resolve(&supplied)
        .await
        .expect("the credentials resolve");
    assert_eq!(resolved.session_token(), Some("token"));
}

#[tokio::test]
async fn environment_supplies_missing_credentials() {
    let resolved = credentials(&[
        ("AWS_ACCESS_KEY_ID", "environment-access"),
        ("AWS_SECRET_ACCESS_KEY", "environment-secret"),
    ])
    .resolve(&params())
    .await
    .expect("the credentials resolve");
    assert_eq!(resolved.access_key_id(), "environment-access");
    assert_eq!(resolved.secret_access_key(), "environment-secret");
}

#[tokio::test]
async fn explicit_keys_win_over_the_environment() {
    let resolved = credentials(&[
        ("AWS_ACCESS_KEY_ID", "environment-access"),
        ("AWS_SECRET_ACCESS_KEY", "environment-secret"),
    ])
    .resolve(&explicit())
    .await
    .expect("the credentials resolve");
    assert_eq!(resolved.access_key_id(), "access");
}

#[tokio::test]
async fn missing_credentials_are_reported() {
    let error = credentials(&[])
        .resolve(&params())
        .await
        .expect_err("no credentials are available");
    assert!(matches!(error, CredentialError::Missing));
}

async fn rejects(supplied: CredentialParams, setting: &'static str) {
    let source = credentials(&[]);
    let validation = source
        .validate(&supplied)
        .expect_err("the flow is unsupported");
    assert!(matches!(
        validation,
        CredentialError::Unsupported { name } if name == setting
    ));
    let resolution = source
        .resolve(&supplied)
        .await
        .expect_err("the flow is unsupported");
    assert!(matches!(
        resolution,
        CredentialError::Unsupported { name } if name == setting
    ));
}

#[tokio::test]
async fn a_profile_flow_is_rejected() {
    let mut supplied = params();
    supplied.profile_name = Some("profile".to_owned());
    rejects(supplied, "sqs_aws_profile_name").await;
}

#[tokio::test]
async fn a_role_flow_is_rejected() {
    let mut supplied = params();
    supplied.role_name = Some("arn:aws:iam::123456789012:role/sqs".to_owned());
    rejects(supplied, "sqs_aws_role_name").await;
}

#[tokio::test]
async fn a_web_identity_flow_is_rejected() {
    let mut supplied = params();
    supplied.web_identity_token = Some(SecretString::from("identity"));
    rejects(supplied, "sqs_aws_web_identity_token").await;
}

#[tokio::test]
async fn an_sts_endpoint_is_rejected() {
    let mut supplied = params();
    supplied.sts_endpoint = Some("https://sts.example.com".to_owned());
    rejects(supplied, "sqs_aws_sts_endpoint").await;
}

#[tokio::test]
async fn an_environment_role_is_rejected_too() {
    let error = credentials(&[("AWS_ROLE_NAME", "arn:aws:iam::123456789012:role/sqs")])
        .validate(&params())
        .expect_err("the environment selects a role");
    assert!(matches!(
        error,
        CredentialError::Unsupported {
            name: "sqs_aws_role_name"
        }
    ));
}
