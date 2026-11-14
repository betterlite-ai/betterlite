#![cfg(test)]

use secrecy::{ExposeSecret as _, SecretString};

use super::super::IAM_AUTH_KEY;
use super::super::caching_key_behaviors::{CredentialKey, vertex_instance_key};

#[test]
fn a_missing_credential_keys_the_cache_with_the_iam_auth_literal() {
    let key = vertex_instance_key(None);
    assert_eq!(key.expose_secret(), IAM_AUTH_KEY);
}

#[test]
fn a_given_credential_keys_the_cache_with_its_own_text() {
    let credentials = SecretString::from(r#"{"type": "service_account"}"#);
    let key = vertex_instance_key(Some(&credentials));
    assert_eq!(key.expose_secret(), credentials.expose_secret());
}

#[test]
fn cache_keys_compare_by_the_credential_text_and_order_it() {
    let first = CredentialKey(SecretString::from("credential-a"));
    let second = CredentialKey(SecretString::from("credential-b"));
    assert_ne!(first, second);
    assert!(first < second);
    assert_eq!(CredentialKey(SecretString::from("credential-a")), first);
    assert_eq!(
        CredentialKey(SecretString::from(IAM_AUTH_KEY)),
        CredentialKey(vertex_instance_key(None))
    );
}
