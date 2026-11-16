#![cfg(test)]

use betterlite_bedrock::s3_access::{TrustedModelCredentials, TrustedValue, frozen_mapping};
use betterlite_context::cloud_storage::CloudStorageError;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use rstest::rstest;
use uuid::Uuid;

use crate::vertex_gcs_access::bucket::BucketCandidates;
use crate::vertex_gcs_access::error::GcsAccessError;

use super::{ReadTarget, STORAGE_BASE_URL, read_url, upload_url};
use crate::vertex_files::FileRequest;
use crate::vertex_files::error::VertexFilesError;

const ID: u128 = 7;
const HYPHENATED: &str = "00000000-0000-0000-0000-000000000007";
const SIMPLE: &str = "00000000000000000000000000000007";
const MANAGED_OBJECT: &str = "litellm-vertex-files/uploads/row-file";

fn candidates(deployment: Option<&str>) -> BucketCandidates {
    BucketCandidates {
        deployment: deployment.map(str::to_owned),
        deployment_alias: None,
        batch_environment: None,
        environment: None,
    }
}

#[test]
fn an_upload_lands_on_the_storage_host() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        None,
        &candidates(Some("my-bucket")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/upload/storage/v1/b/my-bucket/o?uploadType=media&name=litellm-vertex-files%2Fuploads%2F{SIMPLE}-file"
        )
    );
}

#[test]
fn a_custom_api_base_prefixes_the_upload_url() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Some("https://proxy.example"),
        &candidates(Some("my-bucket")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "https://proxy.example/upload/storage/v1/b/my-bucket/o?uploadType=media&name=litellm-vertex-files%2Fuploads%2F{SIMPLE}-file"
        )
    );
}

#[test]
fn a_custom_api_base_keeps_its_trailing_slash() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        Some("https://proxy.example/"),
        &candidates(Some("my-bucket")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "https://proxy.example//upload/storage/v1/b/my-bucket/o?uploadType=media&name=litellm-vertex-files%2Fuploads%2F{SIMPLE}-file"
        )
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn an_unset_api_base_falls_back_to_the_storage_host(#[case] base: Option<&str>) {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Messages,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        base,
        &candidates(Some("my-bucket")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert!(url.starts_with(STORAGE_BASE_URL), "{url}");
}

#[test]
fn the_configured_object_prefix_leads_the_upload_name() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::FineTune,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        None,
        &candidates(Some("my-bucket/exports")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/upload/storage/v1/b/my-bucket/o?uploadType=media&name=exports%2Flitellm-vertex-files%2Fuploads%2F{SIMPLE}-file"
        )
    );
}

#[test]
fn a_passthrough_upload_names_the_object_under_the_passthrough_prefix() {
    let file = FileData::Direct(FileBody::Bytes(b"{}".to_vec()));
    let url = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("vertex_ai/gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        Some("https://proxy.example"),
        &candidates(Some("my-bucket")),
        Uuid::from_u128(ID),
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "https://proxy.example/upload/storage/v1/b/my-bucket/o?uploadType=media&name=litellm-vertex-files%2Fpassthrough%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-pro%2F{HYPHENATED}"
        )
    );
}

#[test]
fn a_batch_custom_endpoint_is_refused_before_the_bucket_lookup() {
    let file = FileData::Direct(FileBody::Bytes(b"{}".to_vec()));
    let failure = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: true,
        },
        None,
        &candidates(None),
        Uuid::from_u128(ID),
    )
    .expect_err("a custom endpoint has no batch surface");
    assert!(matches!(failure, VertexFilesError::CustomEndpointBatch));
    assert_eq!(failure.status_code(), Some(400));
}

#[test]
fn a_custom_endpoint_only_blocks_batch_uploads() {
    let file = FileData::Direct(FileBody::Bytes(b"data".to_vec()));
    let failure = upload_url(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: true,
        },
        None,
        &candidates(None),
        Uuid::from_u128(ID),
    )
    .expect_err("no bucket is configured");
    assert!(matches!(
        failure,
        VertexFilesError::Gcs(GcsAccessError::BucketRequired)
    ));
}

#[test]
fn a_metadata_read_reaches_the_fixed_storage_host() {
    let url = read_url(
        &format!("gs://my-bucket/{MANAGED_OBJECT}"),
        &candidates(Some("my-bucket")),
        &TrustedModelCredentials::default(),
        ReadTarget::Metadata,
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/storage/v1/b/my-bucket/o/litellm-vertex-files%2Fuploads%2Frow-file"
        )
    );
}

#[test]
fn a_content_read_asks_for_the_media() {
    let url = read_url(
        &format!("gs://my-bucket/{MANAGED_OBJECT}"),
        &candidates(Some("my-bucket")),
        &TrustedModelCredentials::default(),
        ReadTarget::Content,
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/storage/v1/b/my-bucket/o/litellm-vertex-files%2Fuploads%2Frow-file?alt=media"
        )
    );
}

#[test]
fn a_configured_object_prefix_stays_in_the_read_path() {
    let url = read_url(
        "gs://my-bucket/exports/litellm-vertex-files/uploads/row-file",
        &candidates(Some("my-bucket/exports")),
        &TrustedModelCredentials::default(),
        ReadTarget::Metadata,
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/storage/v1/b/my-bucket/o/exports%2Flitellm-vertex-files%2Fuploads%2Frow-file"
        )
    );
}

#[test]
fn an_encoded_file_id_reaches_the_same_read_url() {
    let url = read_url(
        "gs://my-bucket/litellm-vertex-files%2Fuploads%2Frow-file",
        &candidates(Some("my-bucket")),
        &TrustedModelCredentials::default(),
        ReadTarget::Metadata,
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "{STORAGE_BASE_URL}/storage/v1/b/my-bucket/o/litellm-vertex-files%2Fuploads%2Frow-file"
        )
    );
}

#[test]
fn a_read_from_another_bucket_is_refused() {
    let failure = read_url(
        "gs://other-bucket/litellm-vertex-files/uploads/row-file",
        &candidates(Some("my-bucket")),
        &TrustedModelCredentials::default(),
        ReadTarget::Metadata,
    )
    .expect_err("the id names a bucket this deployment does not use");
    assert!(matches!(
        failure,
        VertexFilesError::Gcs(GcsAccessError::Storage(CloudStorageError::BucketMismatch))
    ));
}

#[test]
fn an_unmanaged_object_is_refused() {
    let failure = read_url(
        "gs://my-bucket/private/row-file",
        &candidates(Some("my-bucket")),
        &TrustedModelCredentials::default(),
        ReadTarget::Metadata,
    )
    .expect_err("only managed objects are readable");
    assert!(matches!(
        failure,
        VertexFilesError::Gcs(GcsAccessError::Storage(CloudStorageError::Unmanaged))
    ));
}

#[test]
fn a_legacy_object_is_read_when_the_deployment_allows_it() {
    let trusted = frozen_mapping([(
        "allow_legacy_cloud_file_ids".to_owned(),
        TrustedValue::Boolean(true),
    )]);
    let url = read_url(
        "gs://my-bucket/private/row-file",
        &candidates(Some("my-bucket")),
        &trusted,
        ReadTarget::Content,
    )
    .unwrap();
    assert_eq!(
        url,
        format!("{STORAGE_BASE_URL}/storage/v1/b/my-bucket/o/private%2Frow-file?alt=media")
    );
}
