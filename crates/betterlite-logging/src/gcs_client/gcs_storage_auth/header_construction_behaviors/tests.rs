#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use secrecy::SecretString;

use super::{construct_request_headers, sync_construct_request_headers, to_header_map};
use crate::gcs_client::gcs_storage_auth::error::GcsClientError;
use crate::gcs_client::gcs_storage_auth::tests::support::{
    FakeAuth, FakeEnvironment, Harness, PROJECT, RecordingClient, TOKEN, headers,
};
use crate::gcs_client::gcs_storage_auth::{PROJECT_ID_ENV, RequestHeaders, TOKEN_MODEL};
use crate::gcs_mock::auth::{GcsMockAuth, VertexAuthProvider};

fn value<'headers>(headers: &'headers RequestHeaders, name: &str) -> Option<&'headers str> {
    headers.get(name).map(secrecy::ExposeSecret::expose_secret)
}

fn failing_harness() -> Harness {
    Harness {
        environment: FakeEnvironment::new(&[]),
        auth: FakeAuth::failing(),
        client: RecordingClient::responding(200, "{}"),
    }
}

#[tokio::test]
async fn async_headers_carry_the_bearer_token_and_the_json_content_type() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let request_headers = construct_request_headers(&base, "bucket", None, None)
        .await
        .expect("the fake authentication answers");
    assert_eq!(
        value(&request_headers, "Authorization"),
        Some("Bearer original-token")
    );
    assert_eq!(
        value(&request_headers, "Content-Type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn async_headers_use_the_given_vertex_instance() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let instance = Arc::new(GcsMockAuth {
        real: FakeAuth::minting("instance-token"),
        installed: false,
    });
    let request_headers = construct_request_headers(&base, "bucket", None, Some(&instance))
        .await
        .expect("the given instance answers");
    assert_eq!(
        value(&request_headers, "Authorization"),
        Some("Bearer instance-token")
    );
    assert_eq!(
        harness.auth.ensure_count(),
        0,
        "the default instance was asked"
    );
}

#[tokio::test]
async fn async_headers_ask_for_the_service_account_and_no_project() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let service_account = SecretString::from("account-json");
    construct_request_headers(&base, "bucket", Some(&service_account), None)
        .await
        .expect("the fake authentication answers");
    let request = harness.auth.last_ensure();
    assert_eq!(
        request
            .credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("account-json")
    );
    assert_eq!(request.project_id, None);
    assert_eq!(request.custom_llm_provider, VertexAuthProvider::VertexAi);
}

#[tokio::test]
async fn async_headers_mint_the_token_for_the_gcs_bucket_model() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let service_account = SecretString::from("account-json");
    construct_request_headers(&base, "bucket", Some(&service_account), None)
        .await
        .expect("the fake authentication answers");
    let request = harness.auth.last_token_request();
    assert_eq!(request.model, TOKEN_MODEL);
    assert_eq!(
        request
            .auth_header
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some(TOKEN)
    );
    assert_eq!(request.vertex_project.as_deref(), Some(PROJECT));
    assert_eq!(request.vertex_location, None);
    assert!(request.gemini_api_key.is_none());
    assert_eq!(request.stream, None);
    assert_eq!(request.api_base, None);
    assert_eq!(
        request
            .vertex_credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("account-json")
    );
}

#[tokio::test]
async fn async_headers_name_the_bucket_when_the_authentication_fails() {
    let harness = failing_harness();
    let base = harness.build(None);
    let result = construct_request_headers(&base, "broken-bucket", None, None).await;
    assert!(matches!(
        result,
        Err(GcsClientError::Headers { bucket_name, .. }) if bucket_name == "broken-bucket"
    ));
}

#[test]
fn sync_headers_read_the_project_from_the_environment_on_every_call() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let first = sync_construct_request_headers(&base, "bucket", None)
        .expect("the fake authentication answers");
    assert_eq!(harness.auth.last_ensure().project_id, None);
    harness.environment.set(PROJECT_ID_ENV, "project-from-env");
    let second = sync_construct_request_headers(&base, "bucket", None)
        .expect("the fake authentication answers");
    assert_eq!(
        harness.auth.last_ensure().project_id.as_deref(),
        Some("project-from-env")
    );
    harness.environment.remove(PROJECT_ID_ENV);
    let third = sync_construct_request_headers(&base, "bucket", None)
        .expect("the fake authentication answers");
    assert_eq!(harness.auth.last_ensure().project_id, None);
    assert_eq!(
        value(&first, "Authorization"),
        Some("Bearer original-token")
    );
    assert_eq!(value(&second, "Content-Type"), Some("application/json"));
    assert_eq!(
        value(&third, "Authorization"),
        Some("Bearer original-token")
    );
}

#[test]
fn sync_headers_ask_for_the_sync_token_with_the_service_account() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let service_account = SecretString::from("sync-account");
    sync_construct_request_headers(&base, "bucket", Some(&service_account))
        .expect("the fake authentication answers");
    let request = harness.auth.last_token_request();
    assert_eq!(request.model, TOKEN_MODEL);
    assert_eq!(request.vertex_project.as_deref(), Some(PROJECT));
    assert_eq!(harness.auth.sync_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn sync_headers_name_the_bucket_when_the_authentication_fails() {
    let harness = failing_harness();
    let base = harness.build(None);
    let result = sync_construct_request_headers(&base, "broken-bucket", None);
    assert!(matches!(
        result,
        Err(GcsClientError::Headers { bucket_name, .. }) if bucket_name == "broken-bucket"
    ));
}

#[test]
fn a_header_map_carries_the_constructed_headers() {
    let request_headers = headers(&[
        ("Authorization", "Bearer token"),
        ("Content-Type", "application/json"),
    ]);
    let map = to_header_map::<FakeAuth, RecordingClient>(&request_headers)
        .expect("the headers are valid");
    assert_eq!(
        map.get("authorization").map(http::HeaderValue::as_bytes),
        Some(b"Bearer token".as_slice())
    );
    assert_eq!(
        map.get("content-type").map(http::HeaderValue::as_bytes),
        Some(b"application/json".as_slice())
    );
}

#[test]
fn a_header_value_that_is_not_valid_is_an_error() {
    let request_headers = headers(&[("Authorization", "Bearer token\r\nX-Injected: yes")]);
    let result = to_header_map::<FakeAuth, RecordingClient>(&request_headers);
    assert!(matches!(result, Err(GcsClientError::Header(_))));
}

#[test]
fn a_header_name_that_is_not_valid_is_an_error() {
    let request_headers = headers(&[("Bad Header", "value")]);
    let result = to_header_map::<FakeAuth, RecordingClient>(&request_headers);
    assert!(matches!(result, Err(GcsClientError::HeaderName(_))));
}
