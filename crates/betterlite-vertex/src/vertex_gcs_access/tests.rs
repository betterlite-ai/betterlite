#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_bedrock::s3_access::{TrustedModelCredentials, TrustedValue, frozen_mapping};
use betterlite_context::cloud_storage::{
    self, CloudStorageError, Encoding, VERTEX_AI_MANAGED_GCS_PREFIX,
};
use rstest::rstest;
use uuid::Uuid;

use super::bucket::{
    BucketCandidates, GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV, configured_bucket_name,
};
use super::decode::{MAX_FILE_ID_BYTES, decode_vertex_gcs_id};
use super::error::GcsAccessError;
use super::naming::VERTEX_UPLOADS_PREFIX;
use super::parse::{GcsObject, parse_gcs_uri};

fn candidates(
    deployment: Option<&str>,
    alias: Option<&str>,
    batch: Option<&str>,
    environment: Option<&str>,
) -> BucketCandidates {
    BucketCandidates {
        deployment: deployment.map(str::to_owned),
        deployment_alias: alias.map(str::to_owned),
        batch_environment: batch.map(str::to_owned),
        environment: environment.map(str::to_owned),
    }
}

fn allows_legacy(flag: bool) -> TrustedModelCredentials {
    frozen_mapping([(
        "allow_legacy_cloud_file_ids".to_owned(),
        TrustedValue::Boolean(flag),
    )])
}

#[rstest]
#[case::deployment_wins(
    candidates(Some("gcs"), Some("alias"), Some("batch"), Some("env")),
    Ok("gcs")
)]
#[case::deployment_alias(
    candidates(None, Some("alias"), Some("batch"), Some("env")),
    Ok("alias")
)]
#[case::empty_deployment_skipped(
    candidates(Some(""), Some("alias"), Some("batch"), Some("env")),
    Ok("alias")
)]
#[case::batch_environment(candidates(None, None, Some("batch"), Some("env")), Ok("batch"))]
#[case::environment(candidates(None, None, None, Some("env")), Ok("env"))]
#[case::absent(
    candidates(None, None, None, None),
    Err(GcsAccessError::BucketRequired)
)]
#[case::all_empty(
    candidates(Some(""), Some(""), Some(""), Some("")),
    Err(GcsAccessError::BucketRequired)
)]
fn configured_bucket_name_follows_the_litellm_chain(
    #[case] candidates: BucketCandidates,
    #[case] expected: Result<&str, GcsAccessError>,
) {
    match (configured_bucket_name(&candidates), expected) {
        (Ok(bucket), Ok(expected)) => assert_eq!(bucket, expected),
        (Err(error), Err(expected)) => assert_eq!(error, expected),
        (outcome, expectation) => panic!("{outcome:?} does not match {expectation:?}"),
    }
}

#[test]
fn the_bucket_environment_names_are_litellms() {
    assert_eq!(GCS_BATCH_BUCKET_ENV, "GCS_BATCH_BUCKET_NAME");
    assert_eq!(GCS_BUCKET_ENV, "GCS_BUCKET_NAME");
}

#[rstest]
#[case::managed_object(
    "gs://bucket/litellm-vertex-files/input.jsonl",
    "bucket",
    false,
    Ok(("bucket", "litellm-vertex-files%2Finput.jsonl"))
)]
#[case::configured_prefix(
    "gs://bucket/tenant/litellm-vertex-files/input.jsonl",
    "bucket/tenant",
    false,
    Ok(("bucket", "tenant%2Flitellm-vertex-files%2Finput.jsonl"))
)]
#[case::terminal_slash(
    "gs://bucket/litellm-vertex-files/dir/",
    "bucket",
    false,
    Ok(("bucket", "litellm-vertex-files%2Fdir%2F"))
)]
#[case::legacy_trusted(
    "gs://bucket/tenant/legacy.jsonl",
    "bucket/tenant",
    true,
    Ok(("bucket", "tenant%2Flegacy.jsonl"))
)]
#[case::legacy_untrusted(
    "gs://bucket/tenant/legacy.jsonl",
    "bucket/tenant",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::Unmanaged))
)]
#[case::legacy_foreign_prefix(
    "gs://bucket/other/legacy.jsonl",
    "bucket/tenant",
    true,
    Err(GcsAccessError::Storage(CloudStorageError::PrefixMismatch))
)]
#[case::legacy_foreign_bucket(
    "gs://other/tenant/legacy.jsonl",
    "bucket/tenant",
    true,
    Err(GcsAccessError::Storage(CloudStorageError::BucketMismatch))
)]
#[case::foreign_bucket(
    "gs://other/litellm-vertex-files/input.jsonl",
    "bucket",
    true,
    Err(GcsAccessError::Storage(CloudStorageError::BucketMismatch))
)]
#[case::encoded_traversal(
    "gs://bucket/litellm-vertex-files/%2e%2e/secret",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::Segment))
)]
#[case::interior_empty_segment(
    "gs://bucket/litellm-vertex-files/a//b",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::Segment))
)]
#[case::unmanaged_object(
    "gs://bucket/other/input.jsonl",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::Unmanaged))
)]
#[case::wrong_scheme(
    "s3://bucket/litellm-vertex-files/input.jsonl",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::Scheme("gs://".to_owned())))
)]
#[case::missing_object(
    "gs://bucket",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::MissingObject))
)]
#[case::empty_object(
    "gs://bucket/",
    "bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::EmptyObject))
)]
#[case::configured_bucket_with_scheme(
    "gs://bucket/litellm-vertex-files/input.jsonl",
    "gs://bucket",
    false,
    Err(GcsAccessError::Storage(CloudStorageError::BucketUri))
)]
fn parse_gcs_uri_authorizes_the_managed_prefix(
    #[case] file_id: &str,
    #[case] configured_bucket_name: &str,
    #[case] allow_legacy: bool,
    #[case] expected: Result<(&str, &str), GcsAccessError>,
) {
    let trusted = allows_legacy(allow_legacy);
    match (
        parse_gcs_uri(file_id, configured_bucket_name, Some(&trusted)),
        expected,
    ) {
        (Ok(found), Ok((bucket, object))) => {
            assert_eq!(found.bucket, bucket);
            assert_eq!(found.encoded_object, object);
        }
        (Err(error), Err(expected)) => assert_eq!(error, expected),
        (outcome, expectation) => panic!("{outcome:?} does not match {expectation:?}"),
    }
}

#[rstest]
#[case::space_encoded_once(
    "gs://bucket/litellm-vertex-files/a%20b",
    Ok("litellm-vertex-files%2Fa%20b")
)]
#[case::double_encoded_slash(
    "gs://bucket/litellm-vertex-files/a%252Fb",
    Ok("litellm-vertex-files%2Fa%2Fb")
)]
#[case::double_encoded_letter(
    "gs://bucket/litellm-vertex-files/%2541",
    Ok("litellm-vertex-files%2FA")
)]
#[case::encoded_slash_segment(
    "gs://bucket/litellm-vertex-files/%2F",
    Err(GcsAccessError::Storage(CloudStorageError::Segment))
)]
#[case::utf8_path(
    "gs://bucket/litellm-vertex-files/%C3%A9",
    Ok("litellm-vertex-files%2F%C3%A9")
)]
fn parse_gcs_uri_encodes_the_object_once_more(
    #[case] file_id: &str,
    #[case] expected: Result<&str, GcsAccessError>,
) {
    match (parse_gcs_uri(file_id, "bucket", None), expected) {
        (Ok(found), Ok(object)) => assert_eq!(found.encoded_object, object),
        (Err(error), Err(expected)) => assert_eq!(error, expected),
        (outcome, expectation) => panic!("{outcome:?} does not match {expectation:?}"),
    }
}

#[test]
fn default_credentials_do_not_allow_legacy_file_ids() {
    let trusted = TrustedModelCredentials::default();

    assert_eq!(
        parse_gcs_uri("gs://bucket/legacy.jsonl", "bucket", Some(&trusted)).unwrap_err(),
        GcsAccessError::Storage(CloudStorageError::Unmanaged)
    );
}

#[rstest]
#[case::nil_id(
    None,
    Uuid::nil(),
    "litellm-vertex-files/uploads/00000000000000000000000000000000-file"
)]
#[case::sanitized_filename(
    Some("report final?.pdf"),
    Uuid::nil(),
    "litellm-vertex-files/uploads/00000000000000000000000000000000-report_final_.pdf"
)]
#[case::one_id(
    Some("a/b.pdf"),
    Uuid::from_u128(1),
    "litellm-vertex-files/uploads/00000000000000000000000000000001-b.pdf"
)]
fn upload_names_keep_the_managed_prefix_and_the_uuid_hex(
    #[case] filename: Option<&str>,
    #[case] id: Uuid,
    #[case] expected: &str,
) {
    assert_eq!(
        cloud_storage::build_object_name(VERTEX_UPLOADS_PREFIX, filename, "file", id),
        expected
    );
}

#[test]
fn the_uploads_prefix_sits_under_the_managed_prefix() {
    assert!(VERTEX_UPLOADS_PREFIX.starts_with(VERTEX_AI_MANAGED_GCS_PREFIX));
}

#[rstest]
#[case::sanitizes_components("report final?.pdf", "report_final_.pdf")]
#[case::normalizes_slashes("C:\\temp\\file.txt", "file.txt")]
#[case::trims_dots("..hidden..", "hidden")]
#[case::keeps_dots_inside("announcement.v2.pdf", "announcement.v2.pdf")]
#[case::empty_falls_back("", "file")]
fn component_sanitization_matches_the_shared_primitive(
    #[case] filename: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        cloud_storage::sanitize_component(Some(filename), "file"),
        expected
    );
}

#[rstest]
#[case::publisher_model(
    "publishers/google/models/gemini-2.5-pro",
    "publishers/google/models/gemini-2.5-pro"
)]
#[case::reserved_characters(
    "publishers/google/models/gemini 2.5?pro",
    "publishers/google/models/gemini_2.5_pro"
)]
#[case::dot_segments("../publishers//google/./models/x", "publishers/google/models/x")]
#[case::empty_falls_back("", "model")]
fn model_paths_keep_their_segments(#[case] model_path: &str, #[case] expected: &str) {
    assert_eq!(
        cloud_storage::sanitize_path(Some(model_path), "model"),
        expected
    );
}

#[test]
fn the_gcs_encoder_escapes_the_slash() {
    assert_eq!(
        cloud_storage::encode_object_name("litellm-vertex-files/a b", Encoding::Gcs),
        "litellm-vertex-files%2Fa%20b"
    );
    assert_ne!(
        cloud_storage::encode_object_name("litellm-vertex-files/a b", Encoding::S3),
        "litellm-vertex-files%2Fa%20b"
    );
}

#[test]
fn decode_accepts_a_managed_identifier() {
    let decoded = decode_vertex_gcs_id(
        b"gs://bucket/litellm-vertex-files/input.jsonl",
        "bucket",
        None,
    )
    .unwrap();

    assert_eq!(
        decoded,
        GcsObject {
            bucket: "bucket".to_owned(),
            encoded_object: "litellm-vertex-files%2Finput.jsonl".to_owned(),
        }
    );
}

#[test]
fn decode_refuses_bytes_that_are_not_utf8() {
    assert!(matches!(
        decode_vertex_gcs_id(b"gs://bucket/\xff", "bucket", None),
        Err(GcsAccessError::FileIdNotUtf8(_))
    ));
}

#[test]
fn decode_refuses_an_identifier_over_the_limit() {
    let over_limit = vec![b'a'; MAX_FILE_ID_BYTES.saturating_add(1)];

    assert_eq!(
        decode_vertex_gcs_id(&over_limit, "bucket", None).unwrap_err(),
        GcsAccessError::FileIdTooLong {
            limit: MAX_FILE_ID_BYTES
        }
    );
}

#[rstest]
fn replays_the_vertex_gcs_id_corpus(#[files("fuzz/corpus/vertex_gcs_id/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded = decode_vertex_gcs_id(&bytes, "bucket/litellm-vertex-files", None);
}
