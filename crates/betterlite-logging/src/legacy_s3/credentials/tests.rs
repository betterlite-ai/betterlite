#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use crate::s3_transport::signing::CredentialError;

use super::{ConfiguredCredentials, CredentialSource as _};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn credentials(
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    session_token: Option<&str>,
    variables: &[(&str, &str)],
) -> ConfiguredCredentials {
    ConfiguredCredentials {
        environment: Arc::new(environment(variables)),
        access_key_id: access_key_id.map(SecretString::from),
        secret_access_key: secret_access_key.map(SecretString::from),
        session_token: session_token.map(SecretString::from),
    }
}

#[tokio::test]
async fn explicit_keys_win_over_the_environment() {
    let resolved = credentials(
        Some("access"),
        Some("secret"),
        None,
        &[
            ("AWS_ACCESS_KEY_ID", "environment-access"),
            ("AWS_SECRET_ACCESS_KEY", "environment-secret"),
            ("AWS_SESSION_TOKEN", "environment-token"),
        ],
    )
    .resolve()
    .await
    .expect("the credentials resolve");
    assert_eq!(resolved.access_key_id.expose_secret(), "access");
    assert_eq!(resolved.secret_access_key.expose_secret(), "secret");
    assert!(resolved.session_token.is_none());
}

#[tokio::test]
async fn environment_keys_are_the_fallback() {
    let resolved = credentials(
        None,
        None,
        None,
        &[
            ("AWS_ACCESS_KEY_ID", "environment-access"),
            ("AWS_SECRET_ACCESS_KEY", "environment-secret"),
            ("AWS_SESSION_TOKEN", "environment-token"),
        ],
    )
    .resolve()
    .await
    .expect("the credentials resolve");
    assert_eq!(resolved.access_key_id.expose_secret(), "environment-access");
    assert_eq!(
        resolved.secret_access_key.expose_secret(),
        "environment-secret"
    );
    assert_eq!(
        resolved
            .session_token
            .expect("the token is set")
            .expose_secret(),
        "environment-token"
    );
}

#[tokio::test]
async fn the_session_token_travels_with_explicit_keys() {
    let resolved = credentials(Some("access"), Some("secret"), Some("token"), &[])
        .resolve()
        .await
        .expect("the credentials resolve");
    assert_eq!(
        resolved
            .session_token
            .expect("the token is set")
            .expose_secret(),
        "token"
    );
}

#[tokio::test]
async fn a_partial_pair_is_refused() {
    let error = credentials(Some("access"), None, None, &[])
        .resolve()
        .await
        .expect_err("the pair is incomplete");
    assert!(matches!(error, CredentialError::Unavailable));
}

#[tokio::test]
async fn missing_credentials_are_refused() {
    let error = credentials(None, None, None, &[])
        .resolve()
        .await
        .expect_err("no credentials are configured");
    assert!(matches!(error, CredentialError::Unavailable));
}

#[test]
fn debug_output_names_the_struct_without_the_secrets() {
    let rendered = format!(
        "{:?}",
        credentials(Some("hunter2"), Some("swordfish"), Some("token-x"), &[])
    );
    assert!(rendered.contains("ConfiguredCredentials"), "{rendered}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("swordfish"), "{rendered}");
    assert!(!rendered.contains("token-x"), "{rendered}");
}
