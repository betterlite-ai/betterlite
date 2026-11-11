#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use crate::s3_batch::credentials::{ConfiguredKeys, unsupported_flow};
use crate::s3_transport::signing::CredentialSource as _;
use crate::s3_v2::connection::Credentials as ConnectionCredentials;

fn keys(configured: Option<(&str, &str)>, environment: BTreeMap<String, String>) -> ConfiguredKeys {
    let (access_key_id, secret_access_key) = match configured {
        Some((access, secret)) => (
            Some(SecretString::from(access)),
            Some(SecretString::from(secret)),
        ),
        None => (None, None),
    };
    ConfiguredKeys {
        access_key_id,
        secret_access_key,
        session_token: None,
        environment: Arc::new(environment),
    }
}

#[tokio::test]
async fn configured_keys_win_over_the_environment() {
    let source = keys(
        Some(("configured-access", "configured-secret")),
        BTreeMap::from([
            (
                "AWS_ACCESS_KEY_ID".to_owned(),
                "environment-access".to_owned(),
            ),
            (
                "AWS_SECRET_ACCESS_KEY".to_owned(),
                "environment-secret".to_owned(),
            ),
            (
                "AWS_SESSION_TOKEN".to_owned(),
                "environment-token".to_owned(),
            ),
        ]),
    );
    let credentials = source.resolve().await.expect("the keys resolve");
    assert_eq!(
        credentials.access_key_id.expose_secret(),
        "configured-access"
    );
    assert_eq!(
        credentials
            .session_token
            .expect("the token")
            .expose_secret(),
        "environment-token"
    );
}

#[tokio::test]
async fn environment_keys_fill_missing_configured_keys() {
    let source = keys(
        None,
        BTreeMap::from([
            (
                "AWS_ACCESS_KEY_ID".to_owned(),
                "environment-access".to_owned(),
            ),
            (
                "AWS_SECRET_ACCESS_KEY".to_owned(),
                "environment-secret".to_owned(),
            ),
        ]),
    );
    let credentials = source.resolve().await.expect("the keys resolve");
    assert_eq!(
        credentials.secret_access_key.expose_secret(),
        "environment-secret"
    );
    assert!(credentials.session_token.is_none());
}

#[tokio::test]
async fn no_keys_are_unavailable() {
    let source = keys(None, BTreeMap::new());
    source.resolve().await.expect_err("no keys resolve");
}

#[test]
fn unsupported_flow_names_the_setting() {
    let mut credentials = ConnectionCredentials {
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_session_name: None,
        aws_profile_name: Some("profile".to_owned()),
        aws_role_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
    };
    assert_eq!(unsupported_flow(&credentials), Some("s3_aws_profile_name"));
    credentials.aws_profile_name = None;
    credentials.aws_role_name = Some("role".to_owned());
    assert_eq!(unsupported_flow(&credentials), Some("s3_aws_role_name"));
    credentials.aws_role_name = None;
    assert_eq!(unsupported_flow(&credentials), None);
}
