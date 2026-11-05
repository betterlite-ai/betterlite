#![cfg(test)]

use super::decrypt_env_var;
use crate::aws_kms::{AWS_KMS_CLIENT, AwsKmsError, DecryptedValue, tests::replay};
use aws_sdk_kms::config::{Credentials, SharedCredentialsProvider, SharedHttpClient};
use secrecy::ExposeSecret as _;
use std::{collections::BTreeMap, time::Duration};

#[tokio::test]
async fn decrypt_env_var_builds_its_own_client_and_preserves_environment() {
    let source = BTreeMap::from([
        ("AWS_REGION_NAME".to_owned(), "us-east-1".to_owned()),
        (
            "LiTeLlM_SeCrEt_AwS_KmS_TOKEN".to_owned(),
            "aws_kms/aGVsbG8=\n".to_owned(),
        ),
        ("ignored".to_owned(), "plain".to_owned()),
    ]);
    let original = source.clone();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
    let transport = replay(r#"{"Plaintext":"ICBzZWNyZXQgIA=="}"#, 200);
    let result = decrypt_env_var(
        &source,
        SharedHttpClient::new(transport.clone()),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let Some(DecryptedValue::String(secret)) = result.get("TOKEN") else {
        panic!("expected secret");
    };
    assert_eq!(secret.expose_secret(), "secret");
    assert_eq!(result.len(), 1);
    assert_eq!(source, original);
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
    assert!(transport.response.lock().unwrap().is_none());
}

#[tokio::test]
async fn decrypt_env_var_requires_region_even_without_secrets() {
    let result = decrypt_env_var(
        &BTreeMap::new(),
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(result, Err(AwsKmsError::MissingRegion)));
}

#[tokio::test]
async fn decrypt_env_var_selects_value_prefix_and_returns_boolean() {
    let environment = BTreeMap::from([
        ("AWS_REGION_NAME".to_owned(), "us-east-1".to_owned()),
        ("FLAG".to_owned(), "aws_kms/aGVsbG8=".to_owned()),
    ]);
    let transport = replay(r#"{"Plaintext":"VHJ1ZQ=="}"#, 200);
    let result = decrypt_env_var(
        &environment,
        SharedHttpClient::new(transport.clone()),
        Some(SharedCredentialsProvider::new(Credentials::new(
            "test", "test", None, None, "test",
        ))),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert!(matches!(
        result.get("FLAG"),
        Some(DecryptedValue::Boolean(true))
    ));
    assert!(transport.response.lock().unwrap().is_none());
}
