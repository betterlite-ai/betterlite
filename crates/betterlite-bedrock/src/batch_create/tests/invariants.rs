use std::future::ready;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::ExposeSecret;

use super::{SESSION_TOKEN, Sources, fixture_credentials, runtime};
use crate::aws_settings::sources::Credentials;
use crate::batch_create::request::sign::transform_create_batch_request;
use crate::credentials::error::CredentialError;

#[test]
fn bedrock_services_batch_create_credential_identity_isolation() {
    let deployment = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "aws_access_key_id": "DEPLOYKEY", "aws_secret_access_key": "deploy-secret", "aws_region_name": "us-east-1", "aws_role_name": "deploy-role"}"#,
        r#"{"aws_access_key_id": "REQUESTKEY", "aws_secret_access_key": "request-secret", "aws_profile_name": "request-profile"}"#,
        &[],
    );
    assert_signed_as(&deployment, ("DEPLOYKEY", "REQUESTKEY"), |settings| {
        assert_eq!(settings.profile_name, None);
    });
    let request_only = Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r"}"#,
        r#"{"aws_access_key_id": "REQUESTKEY", "aws_secret_access_key": "request-secret", "aws_region_name": "eu-west-1"}"#,
        &[],
    );
    assert_signed_as(&request_only, ("REQUESTKEY", "DEPLOYKEY"), |settings| {
        assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
    });
}

fn assert_signed_as(sources: &Sources, keys: (&'static str, &str), check: fn(&AwsSettings)) {
    let (expected, other) = keys;
    let signed = runtime()
        .block_on(transform_create_batch_request(
            sources.input("m", Some("s3://b/k")),
            |settings, _region| {
                assert_eq!(
                    settings
                        .access_key_id
                        .as_ref()
                        .map(ExposeSecret::expose_secret),
                    Some(expected)
                );
                check(settings);
                ready(Ok::<Credentials, CredentialError>(fixture_credentials(
                    expected, None,
                )))
            },
        ))
        .expect("the identity signs");
    let authorization = signed.headers.get("Authorization").expect("signed");
    assert!(authorization.contains(expected));
    assert!(!authorization.contains(other));
}

#[test]
fn bedrock_services_batch_create_secret_safe_failure() {
    failure_text_keeps_secrets_out();
    successful_debug_keeps_secrets_out();
}

fn secret_fixture() -> Sources {
    Sources::new(
        r#"{"aws_batch_role_arn": "arn:aws:iam::1:role/r", "aws_access_key_id": "LEAKEDACCESS", "aws_secret_access_key": "leaked-secret-key", "aws_session_token": "session-token-fixture"}"#,
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
