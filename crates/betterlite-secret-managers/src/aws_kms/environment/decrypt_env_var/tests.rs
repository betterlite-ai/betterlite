#![cfg(test)]

use super::decrypt_env_var;
use crate::aws_kms::{
    AWS_KMS_CLIENT, AwsKmsError, DecryptedValue,
    tests::{credentials, kms_environment, serve, transport},
};
use secrecy::ExposeSecret as _;
use std::{collections::BTreeMap, time::Duration};
use tokio::net::TcpListener;

#[tokio::test]
async fn decrypt_env_var_builds_its_own_client_and_preserves_environment() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut source = kms_environment(&listener, "us-east-1");
    source.insert(
        "LiTeLlM_SeCrEt_AwS_KmS_TOKEN".to_owned(),
        "aws_kms/aGVsbG8=\n".to_owned(),
    );
    source.insert("ignored".to_owned(), "plain".to_owned());
    let original = source.clone();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
    let (decrypted, request) = tokio::join!(
        decrypt_env_var(
            &source,
            transport(),
            Some(credentials()),
            Duration::from_secs(5)
        ),
        serve(&listener, 200, r#"{"Plaintext":"ICBzZWNyZXQgIA=="}"#)
    );
    let result = decrypted.unwrap();
    let Some(DecryptedValue::String(secret)) = result.get("TOKEN") else {
        panic!("expected secret");
    };
    assert_eq!(secret.expose_secret(), "secret");
    assert_eq!(result.len(), 1);
    assert!(
        request.ends_with(r#"{"CiphertextBlob":"aGVsbG8="}"#),
        "{request}"
    );
    assert_eq!(source, original);
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
}

#[tokio::test]
async fn decrypt_env_var_requires_region_even_without_secrets() {
    let result = decrypt_env_var(&BTreeMap::new(), transport(), None, Duration::from_secs(1)).await;
    assert!(matches!(result, Err(AwsKmsError::MissingRegion)));
}

#[tokio::test]
async fn decrypt_env_var_selects_value_prefix_and_returns_boolean() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut environment = kms_environment(&listener, "us-east-1");
    environment.insert("FLAG".to_owned(), "aws_kms/aGVsbG8=".to_owned());
    let (result, _request) = tokio::join!(
        decrypt_env_var(
            &environment,
            transport(),
            Some(credentials()),
            Duration::from_secs(5)
        ),
        serve(&listener, 200, r#"{"Plaintext":"VHJ1ZQ=="}"#)
    );
    assert!(matches!(
        result.unwrap().get("FLAG"),
        Some(DecryptedValue::Boolean(true))
    ));
}

#[tokio::test]
async fn decrypt_env_var_removes_every_marker_with_python_case_folding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut environment = kms_environment(&listener, "us-east-1");
    environment.insert(
        "litellm_secret_aws_kms_A_LITELLM_SECRET_AWS_KMS_B".to_owned(),
        "aGVsbG8=".to_owned(),
    );
    environment.insert(
        "LITELLM_SECRET_AWS_\u{212a}MS_KELVIN".to_owned(),
        "aGVsbG8=".to_owned(),
    );
    let responses = async {
        serve(&listener, 200, r#"{"Plaintext":"b25l"}"#).await;
        serve(&listener, 200, r#"{"Plaintext":"dHdv"}"#).await;
    };
    let (decrypted, ()) = tokio::join!(
        decrypt_env_var(
            &environment,
            transport(),
            Some(credentials()),
            Duration::from_secs(5)
        ),
        responses
    );
    assert_eq!(
        decrypted
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["A_B", "KELVIN"]
    );
}
