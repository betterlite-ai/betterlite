#![cfg(test)]

use std::time::Duration;

use betterlite_bedrock::s3_access::{TrustedModelCredentials, TrustedValue, frozen_mapping};
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileContentRequest;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::vertex_batch_download::records::ConversionContext;
use crate::vertex_file_content::settings::ReadDeployment;

mod support;

use betterlite_logging::gcs_client::gcs_storage_auth::SERVICE_ACCOUNT_ENV;

use self::support::{
    BUCKET, FILE_ID, Harness, LEGACY_FILE_ID, LEGACY_STORAGE_URL, MODEL_FILE_ID, MODEL_STORAGE_URL,
    SERVICE_ACCOUNT_PATH, SPACED_FILE_ID, SPACED_STORAGE_URL, STORAGE_URL, Step, content_request,
    token_for,
};
use super::ContentRead;
use super::error::FileContentError;
use super::file_content;
use crate::vertex_gcs_access::bucket::{GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV};

const NOW: i64 = 1_700_000_000;
const ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2,"trafficType":"ON_DEMAND"}}}"#;

fn context() -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now: NOW,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fn deployment_of(
    gcs_bucket_name: Option<Json>,
    bucket_name: Option<Json>,
    vertex_credentials: Option<Json>,
) -> ReadDeployment {
    ReadDeployment {
        gcs_bucket_name,
        bucket_name,
        vertex_credentials,
    }
}

fn trusted_record(legacy: bool) -> TrustedModelCredentials {
    frozen_mapping([(
        "allow_legacy_cloud_file_ids".to_owned(),
        TrustedValue::Boolean(legacy),
    )])
}

fn content_read<'read>(
    request: &'read FileContentRequest,
    deployment: &'read ReadDeployment,
    credentials: Option<&'read Json>,
    trusted: &'read TrustedModelCredentials,
) -> ContentRead<'read> {
    ContentRead {
        request,
        deployment,
        credentials,
        trusted,
        disabled: false,
    }
}

fn header<'headers>(headers: &'headers IndexMap<String, String>, name: &str) -> &'headers str {
    headers
        .get(name)
        .expect("the read declares the header")
        .as_str()
}

#[tokio::test]
async fn authorize_before_get() {
    // the batch environment names the bucket, so the read resolver is the only path to it
    let harness = Harness::with_env(&[(GCS_BATCH_BUCKET_ENV, BUCKET)]);
    let base = harness.base();
    let request = content_request(FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("the read succeeds");
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"{}".to_vec());
    assert_eq!(
        harness.steps(),
        vec![
            Step::Authorized(None),
            Step::Authorized(None),
            Step::Sent(STORAGE_URL.to_owned()),
        ]
    );
    assert_eq!(
        harness.authorization(0),
        format!("Bearer {}", token_for(None))
    );
    let denied = Harness::with_env(&[(GCS_BATCH_BUCKET_ENV, BUCKET)]);
    denied.set_auth_failure(true);
    let denied_base = denied.base();
    let outcome = file_content(&denied_base, &read, &mut state).await;
    assert!(matches!(outcome, Err(FileContentError::Gcs(_))));
    assert!(denied.requests().is_empty());
}

#[tokio::test]
async fn legacy_authorization() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]);
    let base = harness.base();
    let request = content_request(LEGACY_FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(true);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("a deployment that allows legacy ids reads the object");
    assert_eq!(content.status, 200);
    assert_eq!(
        harness.steps(),
        vec![
            Step::Authorized(None),
            Step::Authorized(None),
            Step::Sent(LEGACY_STORAGE_URL.to_owned()),
        ]
    );
    let untrusted = trusted_record(false);
    let refused = content_read(&request, &deployment, None, &untrusted);
    let error = file_content(&base, &refused, &mut state)
        .await
        .expect_err("a deployment without the switch refuses the legacy object");
    assert!(matches!(error, FileContentError::FileId(_)));
    assert_eq!(harness.steps().len(), 3);
}

#[tokio::test]
async fn credential_partition() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]);
    let base = harness.base();
    let request = content_request(FILE_ID);
    let trusted = trusted_record(false);
    let mut state = context();
    let mut fields = IndexMap::new();
    fields.insert(
        "client_email".to_owned(),
        Json::Text("a@example".to_owned()),
    );
    let dictionary = r#"{"client_email": "a@example"}"#;
    let first_deployment = deployment_of(None, None, Some(Json::Object(fields)));
    let first_read = content_read(&request, &first_deployment, None, &trusted);
    file_content(&base, &first_read, &mut state)
        .await
        .expect("the first read succeeds");
    let argument = Json::Text("second-sa".to_owned());
    let second_deployment = deployment_of(None, None, None);
    let second_read = content_read(&request, &second_deployment, Some(&argument), &trusted);
    file_content(&base, &second_read, &mut state)
        .await
        .expect("the second read succeeds");
    assert_eq!(
        harness.steps(),
        vec![
            Step::Authorized(Some(dictionary.to_owned())),
            Step::Authorized(Some(dictionary.to_owned())),
            Step::Sent(STORAGE_URL.to_owned()),
            Step::Authorized(Some("second-sa".to_owned())),
            Step::Authorized(Some("second-sa".to_owned())),
            Step::Sent(STORAGE_URL.to_owned()),
        ]
    );
    assert_eq!(
        harness.authorization(0),
        format!("Bearer {}", token_for(Some(dictionary)))
    );
    assert_eq!(
        harness.authorization(1),
        format!("Bearer {}", token_for(Some("second-sa")))
    );
}

#[tokio::test]
async fn blank_dynamic_values_use_the_base_environment() {
    let harness = Harness::with_env(&[
        (GCS_BUCKET_ENV, BUCKET),
        (SERVICE_ACCOUNT_ENV, SERVICE_ACCOUNT_PATH),
    ]);
    let base = harness.base();
    let request = content_request(FILE_ID);
    let deployment = deployment_of(None, Some(Json::Text(String::new())), None);
    let blank_argument = Json::Text(String::new());
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, Some(&blank_argument), &trusted);
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("the blank values fall back to the base environment");
    assert_eq!(content.status, 200);
    assert_eq!(
        harness.steps(),
        vec![
            Step::Authorized(Some(SERVICE_ACCOUNT_PATH.to_owned())),
            Step::Authorized(Some(SERVICE_ACCOUNT_PATH.to_owned())),
            Step::Sent(STORAGE_URL.to_owned()),
        ]
    );
}

#[tokio::test]
async fn missing_file_id() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]);
    let base = harness.base();
    let request = content_request("");
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let error = file_content(&base, &read, &mut state)
        .await
        .expect_err("an empty file id is refused");
    assert_eq!(
        error.to_string(),
        "file_id is required in file_content_request"
    );
    assert!(harness.steps().is_empty());
}

#[tokio::test]
async fn empty_content() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]).responding(200, b"");
    let base = harness.base();
    // the handler never reads a request timeout, so a set one changes nothing about the read
    let mut request = content_request(FILE_ID);
    request.timeout = Some(Duration::from_secs(5));
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("zero bytes are a successful read");
    assert_eq!(content.status, 200);
    assert!(content.body.is_empty());
    assert_eq!(
        header(&content.headers, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(header(&content.headers, "content-length"), "0");
    assert_eq!(
        harness.steps().last(),
        Some(&Step::Sent(STORAGE_URL.to_owned()))
    );
}

#[tokio::test]
async fn failed_download() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]).responding(500, b"nope");
    let base = harness.base();
    let request = content_request(SPACED_FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let error = file_content(&base, &read, &mut state)
        .await
        .expect_err("a refused download is an error");
    assert_eq!(
        error.to_string(),
        "Failed to download file from GCS: gs://bucket/litellm-vertex-files/a b.jsonl"
    );
    assert_eq!(
        harness.steps().last(),
        Some(&Step::Sent(SPACED_STORAGE_URL.to_owned()))
    );
    let broken = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]);
    broken.set_transport_failure(true);
    let broken_base = broken.base();
    let outcome = file_content(&broken_base, &read, &mut state).await;
    assert!(matches!(outcome, Err(FileContentError::Download { .. })));
}

#[tokio::test]
async fn transformed_output() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]).responding(200, ROW.as_bytes());
    let base = harness.base();
    let request = content_request(MODEL_FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("the read converts the row");
    assert_eq!(content.status, 200);
    assert_ne!(content.body, ROW.as_bytes());
    let body: Json = serde_json::from_slice(&content.body).expect("the converted row is JSON");
    let Json::Object(fields) = &body else {
        panic!("a converted row is an object");
    };
    assert!(fields.contains_key("custom_id"));
    assert_eq!(
        header(&content.headers, "content-length"),
        content.body.len().to_string().as_str()
    );
    assert_eq!(
        header(&content.headers, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(
        harness.steps().last(),
        Some(&Step::Sent(MODEL_STORAGE_URL.to_owned()))
    );
}

#[tokio::test]
async fn transformation_disabled() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]).responding(200, ROW.as_bytes());
    let base = harness.base();
    let request = content_request(MODEL_FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let mut read = content_read(&request, &deployment, None, &trusted);
    read.disabled = true;
    let mut state = context();
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("a disabled read returns the stored row");
    assert_eq!(content.status, 200);
    assert_eq!(content.body, ROW.as_bytes().to_vec());
    assert_eq!(
        header(&content.headers, "content-length"),
        ROW.len().to_string().as_str()
    );
    assert_eq!(
        header(&content.headers, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(
        harness.steps().last(),
        Some(&Step::Sent(MODEL_STORAGE_URL.to_owned()))
    );
}

#[tokio::test]
async fn cancel_read() {
    let harness = Harness::with_env(&[(GCS_BUCKET_ENV, BUCKET)]);
    harness.set_auth_hang(true);
    let base = harness.base();
    let request = content_request(FILE_ID);
    let deployment = deployment_of(None, None, None);
    let trusted = trusted_record(false);
    let read = content_read(&request, &deployment, None, &trusted);
    let mut state = context();
    let cancelled = tokio::time::timeout(
        Duration::from_millis(50),
        file_content(&base, &read, &mut state),
    )
    .await;
    assert!(cancelled.is_err(), "the read was cancelled");
    assert!(harness.requests().is_empty());
    harness.set_auth_hang(false);
    let content = file_content(&base, &read, &mut state)
        .await
        .expect("the next read completes");
    assert_eq!(content.status, 200);
    assert_eq!(harness.requests().len(), 1);
}
