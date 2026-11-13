#![cfg(test)]

use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use super::get_or_create_vertex_instance;
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::tests::support::{FakeAuth, Harness, ensure_request};
use crate::gcs_mock::auth::{MOCK_TOKEN, VertexAuth as _};

#[tokio::test]
async fn one_credential_gets_one_instance_however_often_it_is_asked() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let credentials = SecretString::from("account-a");
    let first = get_or_create_vertex_instance(&base, Some(&credentials))
        .await
        .expect("the fake authentication answers");
    let second = get_or_create_vertex_instance(&base, Some(&credentials))
        .await
        .expect("the fake authentication answers");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(harness.auth.ensure_count(), 1);
}

#[tokio::test]
async fn different_credentials_get_different_instances() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let first = get_or_create_vertex_instance(&base, Some(&SecretString::from("account-a")))
        .await
        .expect("the fake authentication answers");
    let second = get_or_create_vertex_instance(&base, Some(&SecretString::from("account-b")))
        .await
        .expect("the fake authentication answers");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(harness.auth.ensure_count(), 2);
    assert_eq!(base.vertex_instances.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_missing_credential_is_one_instance_of_its_own() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let first = get_or_create_vertex_instance(&base, None)
        .await
        .expect("the fake authentication answers");
    let second = get_or_create_vertex_instance(&base, None)
        .await
        .expect("the fake authentication answers");
    let credentialed = get_or_create_vertex_instance(&base, Some(&SecretString::from("account")))
        .await
        .expect("the fake authentication answers");
    assert!(Arc::ptr_eq(&first, &second));
    assert!(!Arc::ptr_eq(&first, &credentialed));
    assert_eq!(harness.auth.ensure_count(), 2);
}

#[tokio::test]
async fn a_new_instance_is_warmed_without_a_project() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    get_or_create_vertex_instance(&base, Some(&SecretString::from("account-a")))
        .await
        .expect("the fake authentication answers");
    let request = harness.auth.last_ensure();
    assert_eq!(
        request
            .credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("account-a")
    );
    assert_eq!(request.project_id, None);
}

#[tokio::test]
async fn an_authentication_failure_is_reported_and_caches_nothing() {
    let mut harness = Harness::new(&[]);
    harness.auth = FakeAuth::failing();
    let base = harness.build(None);
    let result = get_or_create_vertex_instance(&base, Some(&SecretString::from("account-a"))).await;
    assert!(matches!(result, Err(GcsClientError::Token(_))));
    assert_eq!(base.vertex_instances.lock().unwrap().len(), 0);
    let retried =
        get_or_create_vertex_instance(&base, Some(&SecretString::from("account-a"))).await;
    assert!(matches!(retried, Err(GcsClientError::Token(_))));
    assert_eq!(harness.auth.ensure_count(), 2);
}

#[tokio::test]
async fn a_new_instance_keeps_the_mock_authentication() {
    let harness = Harness::new(&[("GCS_MOCK", "true"), ("GCS_MOCK_LATENCY_MS", "0")]);
    let base = harness.build(None);
    let instance = get_or_create_vertex_instance(&base, Some(&SecretString::from("account-a")))
        .await
        .expect("the mock authentication answers");
    assert_eq!(
        harness.auth.ensure_count(),
        0,
        "the real authentication was asked"
    );
    let token = instance
        .ensure_access_token_async(ensure_request())
        .await
        .expect("the mock authentication answers");
    assert_eq!(token.token.expose_secret(), MOCK_TOKEN);
}
