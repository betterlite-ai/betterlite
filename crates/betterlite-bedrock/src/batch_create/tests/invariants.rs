use std::future::ready;

use secrecy::ExposeSecret;

use super::{SESSION_TOKEN, Sources, fixture_credentials, runtime};
use crate::aws_settings::sources::Credentials;
use crate::batch_create::request::sign::transform_create_batch_request;
use crate::credentials::error::CredentialError;

#[test]
fn bedrock_services_batch_create_credential_identity_isolation() {
    deployment_identity_is_isolated();
    request_identity_is_isolated();
}

fn deployment_identity_is_isolated() {
    let deployment = Sources::new(
        r#"{"aws_access_key_id": "DEPLOYKEY", "aws_secret_access_key": "deploy-secret", "aws_region_name": "us-east-1", "aws_role_name": "deploy-role"}"#,
        r#"{"aws_access_key_id": "REQUESTKEY", "aws_secret_access_key": "request-secret", "aws_profile_name": "request-profile"}"#,
        &[],
    );
    let signed = runtime()
        .block_on(transform_create_batch_request(
            deployment.input("m", Some("s3://b/k")),
            |settings, _region| {
                assert_eq!(
                    settings
                        .access_key_id
                        .as_ref()
                        .map(ExposeSecret::expose_secret),
                    Some("DEPLOYKEY")
                );
                assert_eq!(settings.profile_name, None);
                ready(Ok::<Credentials, CredentialError>(fixture_credentials(
                    "DEPLOYKEY",
                    None,
                )))
            },
        ))
        .expect("the deployment identity signs");
    let authorization = signed.headers.get("Authorization").expect("signed");
    assert!(authorization.contains("DEPLOYKEY"));
    assert!(!authorization.contains("REQUESTKEY"));
}

fn request_identity_is_isolated() {
    let request_only = Sources::new(
        "{}",
        r#"{"aws_access_key_id": "REQUESTKEY", "aws_secret_access_key": "request-secret", "aws_region_name": "eu-west-1"}"#,
        &[],
    );
    let signed = runtime()
        .block_on(transform_create_batch_request(
            request_only.input("m", Some("s3://b/k")),
            |settings, _region| {
                assert_eq!(
                    settings
                        .access_key_id
                        .as_ref()
                        .map(ExposeSecret::expose_secret),
                    Some("REQUESTKEY")
                );
                assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
                ready(Ok::<Credentials, CredentialError>(fixture_credentials(
                    "REQUESTKEY",
                    None,
                )))
            },
        ))
        .expect("the request identity signs");
    let authorization = signed.headers.get("Authorization").expect("signed");
    assert!(authorization.contains("REQUESTKEY"));
    assert!(!authorization.contains("DEPLOYKEY"));
}

#[test]
fn bedrock_services_batch_create_secret_safe_failure() {
    failure_text_keeps_secrets_out();
    successful_debug_keeps_secrets_out();
}

fn secret_fixture() -> Sources {
    Sources::new(
        r#"{"aws_access_key_id": "LEAKEDACCESS", "aws_secret_access_key": "leaked-secret-key", "aws_session_token": "session-token-fixture"}"#,
        "{}",
        &[],
    )
}

fn failure_text_keeps_secrets_out() {
    let sources = secret_fixture();
    let error = runtime()
        .block_on(transform_create_batch_request(
            sources.input("m", Some("s3://b/k")),
            |_settings, _region| {
                ready(Err::<Credentials, CredentialError>(
                    CredentialError::MissingCredentials,
                ))
            },
        ))
        .expect_err("the credential lookup fails");
    let rendered = format!("{error} {error:?}");
    for secret in ["LEAKEDACCESS", "leaked-secret-key", SESSION_TOKEN] {
        assert!(!rendered.contains(secret), "{secret} reached the failure");
    }
}

fn successful_debug_keeps_secrets_out() {
    let sources = secret_fixture();
    let signed = runtime()
        .block_on(transform_create_batch_request(
            sources.input("m", Some("s3://b/k")),
            |_settings, _region| {
                ready(Ok::<Credentials, CredentialError>(fixture_credentials(
                    "LEAKEDACCESS",
                    Some(SESSION_TOKEN),
                )))
            },
        ))
        .expect("the request signs");
    let rendered = format!("{signed:?} {}", signed.data);
    for secret in ["LEAKEDACCESS", "leaked-secret-key", SESSION_TOKEN] {
        assert!(!rendered.contains(secret), "{secret} reached Debug");
    }
    assert!(rendered.contains("model-invocation-job"));
    assert!(
        signed
            .headers
            .get("X-Amz-Security-Token")
            .is_some_and(|value| value == SESSION_TOKEN)
    );
}
