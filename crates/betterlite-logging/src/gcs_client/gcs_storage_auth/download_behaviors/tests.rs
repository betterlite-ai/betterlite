#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::download_gcs_object;
use crate::gcs_client::gcs_storage_auth::tests::support::{FakeAuth, Harness, RecordingClient};
use crate::gcs_mock::request::GcsRequest;
use crate::types::gcs_storage_auth::type_definitions::GcsLoggingKwargs;

const DOWNLOAD_URL: &str = "https://storage.googleapis.com/storage/v1/b/bucket/o/object?alt=media";

fn harness_with(client: RecordingClient) -> Harness {
    let mut harness = Harness::new(&[("GCS_BUCKET_NAME", "bucket")]);
    harness.client = client;
    harness
}

fn dynamic_bucket(name: &str) -> GcsLoggingKwargs {
    let mut fields = IndexMap::new();
    fields.insert("gcs_bucket_name".to_owned(), Json::Text(name.to_owned()));
    GcsLoggingKwargs {
        dynamic_params: Some(Json::Object(fields)),
    }
}

fn sent_get(harness: &Harness) -> (String, http::HeaderMap) {
    let request = harness.last_request();
    let url = request.url().to_owned();
    let GcsRequest::Get(read) = request else {
        panic!("the download sent a GET");
    };
    (url, read.headers)
}

#[tokio::test]
async fn a_200_download_returns_the_body_bytes() {
    let harness = harness_with(RecordingClient::responding(200, "stored-bytes"));
    let base = harness.build(None);
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, Some(b"stored-bytes".to_vec()));
    let (url, headers) = sent_get(&harness);
    assert_eq!(url, DOWNLOAD_URL);
    assert_eq!(
        headers
            .get("authorization")
            .map(|value| value.to_str().unwrap_or_default()),
        Some("Bearer original-token")
    );
}

#[tokio::test]
async fn a_non_200_download_returns_none() {
    let harness = harness_with(RecordingClient::responding(404, "missing"));
    let base = harness.build(None);
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, None);
    assert_eq!(harness.requests().len(), 1);
}

#[tokio::test]
async fn a_download_of_a_folder_bucket_puts_the_folder_in_the_object_path() {
    let mut harness = Harness::new(&[("GCS_BUCKET_NAME", "my-bucket/folder/dev")]);
    harness.client = RecordingClient::responding(200, "{}");
    let base = harness.build(None);
    download_gcs_object(&base, "my-object", &GcsLoggingKwargs::default()).await;
    let (url, _headers) = sent_get(&harness);
    assert_eq!(
        url,
        "https://storage.googleapis.com/storage/v1/b/my-bucket/o/folder%2Fdev%2Fmy-object?alt=media"
    );
}

#[tokio::test]
async fn a_download_encodes_the_object_name() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    download_gcs_object(&base, "a b/c.json", &GcsLoggingKwargs::default()).await;
    let (url, _headers) = sent_get(&harness);
    assert_eq!(
        url,
        "https://storage.googleapis.com/storage/v1/b/bucket/o/a%20b%2Fc.json?alt=media"
    );
}

#[tokio::test]
async fn a_download_uses_the_dynamic_bucket() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(None);
    download_gcs_object(&base, "object", &dynamic_bucket("dyn-bucket")).await;
    let (url, _headers) = sent_get(&harness);
    assert_eq!(
        url,
        "https://storage.googleapis.com/storage/v1/b/dyn-bucket/o/object?alt=media"
    );
}

#[tokio::test]
async fn a_failed_transport_returns_none() {
    let harness = harness_with(RecordingClient::failing());
    let base = harness.build(None);
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, None);
}

#[tokio::test]
async fn an_authentication_failure_returns_none() {
    let mut harness = Harness::new(&[("GCS_BUCKET_NAME", "bucket")]);
    harness.auth = FakeAuth::failing();
    let base = harness.build(None);
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, None);
    assert!(
        harness.requests().is_empty(),
        "the client was asked for the object"
    );
}

#[tokio::test]
async fn a_download_without_a_bucket_name_returns_none() {
    let harness = Harness::new(&[]);
    let base = harness.build(None);
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, None);
}

#[tokio::test]
async fn a_download_with_a_bucket_that_is_not_a_name_returns_none() {
    let harness = harness_with(RecordingClient::responding(200, "{}"));
    let base = harness.build(Some("gs://bucket"));
    let body = download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    assert_eq!(body, None);
    assert!(
        harness.requests().is_empty(),
        "the client was asked for the object"
    );
}

#[tokio::test]
async fn the_download_carries_the_service_account_into_the_authentication() {
    let mut harness = Harness::new(&[
        ("GCS_BUCKET_NAME", "bucket"),
        ("GCS_PATH_SERVICE_ACCOUNT", "instance-account"),
    ]);
    harness.client = RecordingClient::responding(200, "{}");
    let base = harness.build(None);
    download_gcs_object(&base, "object", &GcsLoggingKwargs::default()).await;
    let request = harness.auth.last_ensure();
    assert_eq!(
        request
            .credentials
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("instance-account")
    );
}
