#![cfg(test)]

use std::sync::Arc;

use secrecy::ExposeSecret as _;

use super::initialize;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::tests::support::{
    FakeAuth, FakeEnvironment, Harness, RecordingClient, TOKEN, ensure_request, get,
};
use crate::gcs_mock::auth::{MOCK_TOKEN, VertexAuth as _};
use crate::gcs_mock::client::GcsHttpClient as _;
use crate::gcs_mock::payload::MOCK_LOGGING_PAYLOAD;

const MOCKED_URL: &str = "https://storage.googleapis.com/storage/v1/b/bucket/o/object?alt=media";

#[test]
fn initialize_reads_the_bucket_name_from_the_environment() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let base = harness.build(None);
    assert_eq!(base.bucket_name.as_deref(), Some("env-bucket"));
}

#[test]
fn initialize_prefers_the_given_bucket_name_and_keeps_an_empty_one() {
    let harness = Harness::new(&[("GCS_BUCKET_NAME", "env-bucket")]);
    let given = harness.build(Some("given-bucket"));
    assert_eq!(given.bucket_name.as_deref(), Some("given-bucket"));
    let empty = harness.build(Some(""));
    assert_eq!(empty.bucket_name.as_deref(), Some(""));
}

#[test]
fn initialize_reads_the_service_account_path_from_the_environment() {
    let harness = Harness::new(&[("GCS_PATH_SERVICE_ACCOUNT", "/secrets/account.json")]);
    let base = harness.build(None);
    assert_eq!(
        base.path_service_account
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("/secrets/account.json")
    );
    let absent = Harness::new(&[]).build(None);
    assert!(absent.path_service_account.is_none());
}

#[tokio::test]
async fn initialize_installs_the_mock_transport_and_authentication_when_enabled() {
    let harness = Harness::new(&[
        ("GCS_BUCKET_NAME", "bucket"),
        ("GCS_MOCK", "true"),
        ("GCS_MOCK_LATENCY_MS", "0"),
    ]);
    let base = harness.build(None);
    assert!(base.is_mock_mode);
    let response = base
        .client
        .send(get(MOCKED_URL))
        .await
        .expect("the mock client answers a storage URL");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body,
        serde_json::to_vec(&MOCK_LOGGING_PAYLOAD).expect("the canned payload serializes")
    );
    assert!(harness.requests().is_empty(), "the real client was reached");
    let token = base
        .auth
        .ensure_access_token_async(ensure_request())
        .await
        .expect("the mock authentication answers");
    assert_eq!(token.token.expose_secret(), MOCK_TOKEN);
}

#[tokio::test]
async fn initialize_keeps_the_mock_and_the_real_authentication_off_when_disabled() {
    let harness = Harness::new(&[("GCS_MOCK", "false")]);
    let base = harness.build(None);
    assert!(!base.is_mock_mode);
    let response = base
        .client
        .send(get(MOCKED_URL))
        .await
        .expect("the real client answers");
    assert_eq!(response.status, 200);
    assert_eq!(harness.requests().len(), 1);
    let token = base
        .auth
        .ensure_access_token_async(ensure_request())
        .await
        .expect("the real authentication answers");
    assert_eq!(token.token.expose_secret(), TOKEN);
}

#[test]
fn initialize_rejects_a_mock_variable_that_is_not_a_bool() {
    let environment: Arc<dyn EnvironmentSource> =
        Arc::new(FakeEnvironment::new(&[("GCS_MOCK", "maybe")]));
    let result = initialize(environment, None, FakeAuth::new(), RecordingClient::new());
    assert!(matches!(result, Err(GcsClientError::Mock(_))));
}

#[tokio::test]
async fn initialize_keeps_the_mock_decision_for_the_lifetime_of_the_instance() {
    let harness = Harness::new(&[("GCS_MOCK", "true"), ("GCS_MOCK_LATENCY_MS", "0")]);
    let base = harness.build(None);
    harness.environment.set("GCS_MOCK", "false");
    assert!(base.is_mock_mode);
    let response = base
        .client
        .send(get(MOCKED_URL))
        .await
        .expect("the mock client answers");
    assert_eq!(
        response.body,
        serde_json::to_vec(&MOCK_LOGGING_PAYLOAD).expect("the canned payload serializes")
    );
    assert!(harness.requests().is_empty(), "the real client was reached");
}
