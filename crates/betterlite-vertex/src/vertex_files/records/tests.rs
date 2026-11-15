#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use jiff::tz::TimeZone;
use rstest::rstest;

use super::{METADATA_BODY_LIMIT, MetadataShape, decode_vertex_file_metadata, deleted_file};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_files::error::VertexFilesError;

const OBJECT_PATH: &str = "litellm-vertex-files/uploads/rows.jsonl";
const CREATED: &str = "2024-12-04T21:53:12.120184Z";
const CREATED_SECONDS: i64 = 1_733_349_192;
const CREATED_JSON: &str = r#""timeCreated":"2024-12-04T21:53:12.120184Z""#;

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::UTC,
    }
}

fn upload_body(purpose: &str) -> Vec<u8> {
    format!(
        r#"{{"kind":"storage#object","id":"my-bucket/{OBJECT_PATH}/1700000000000000","name":"{OBJECT_PATH}","size":"42","timeCreated":"{CREATED}"{purpose}}}"#
    )
    .into_bytes()
}

fn retrieve_body(purpose: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"my-bucket/{OBJECT_PATH}/1700000000000000","name":"{OBJECT_PATH}","size":"42","timeCreated":"{CREATED}"{purpose}}}"#
    )
    .into_bytes()
}

fn check_corpus_seed(name: &str, bytes: &[u8]) {
    if name.starts_with("valid-upload") {
        decode_vertex_file_metadata(bytes, MetadataShape::Upload, &utc())
            .expect("this seed is an upload reply");
    }
    if name.starts_with("valid-retrieve") {
        decode_vertex_file_metadata(bytes, MetadataShape::Retrieve, &utc())
            .expect("this seed is a retrieve reply");
    }
    if name.starts_with("invalid") {
        let upload = decode_vertex_file_metadata(bytes, MetadataShape::Upload, &utc());
        let retrieve = decode_vertex_file_metadata(bytes, MetadataShape::Retrieve, &utc());
        assert!(upload.is_err() && retrieve.is_err(), "{name} decodes");
    }
    if let Ok(text) = str::from_utf8(bytes) {
        if name.starts_with("unusable-delete-url") {
            assert_eq!(deleted_file(Some(text)).id, "deleted", "{name}");
        }
    }
}

#[test]
fn an_upload_reply_reads_the_root_purpose() {
    let file = decode_vertex_file_metadata(
        &upload_body(r#","purpose":"fine-tune""#),
        MetadataShape::Upload,
        &utc(),
    )
    .unwrap();
    assert_eq!(
        file,
        FileObject {
            id: format!("gs://my-bucket/{OBJECT_PATH}"),
            bytes: 42,
            created_at: CREATED_SECONDS,
            filename: OBJECT_PATH.to_owned(),
            object: FileKind::File,
            purpose: FilesPurpose::FineTune,
            status: Some(FileStatus::Uploaded),
            expires_at: None,
            status_details: None,
        }
    );
}

#[test]
fn a_retrieve_reply_reads_the_nested_purpose() {
    let file = decode_vertex_file_metadata(
        &retrieve_body(r#","metadata":{"purpose":"batch"}"#),
        MetadataShape::Retrieve,
        &utc(),
    )
    .unwrap();
    assert_eq!(file.purpose, FilesPurpose::Batch);
    assert_eq!(file.status, Some(FileStatus::Processed));
    assert_eq!(file.filename, OBJECT_PATH);
    assert_eq!(file.id, format!("gs://my-bucket/{OBJECT_PATH}"));
}

#[test]
fn a_retrieve_reply_ignores_a_root_purpose() {
    let file = decode_vertex_file_metadata(
        &retrieve_body(r#","purpose":"vision","metadata":{}"#),
        MetadataShape::Retrieve,
        &utc(),
    )
    .unwrap();
    assert_eq!(file.purpose, FilesPurpose::Batch);
}

#[test]
fn an_upload_reply_ignores_a_nested_purpose() {
    let file = decode_vertex_file_metadata(
        &upload_body(r#","metadata":{"purpose":"vision"}"#),
        MetadataShape::Upload,
        &utc(),
    )
    .unwrap();
    assert_eq!(file.purpose, FilesPurpose::Batch);
}

#[test]
fn a_reply_without_a_purpose_or_size_defaults_to_batch_and_zero() {
    let body = format!(r#"{{"id":"my-bucket/o/1",{CREATED_JSON}}}"#);
    let file = decode_vertex_file_metadata(body.as_bytes(), MetadataShape::Upload, &utc()).unwrap();
    assert_eq!(file.purpose, FilesPurpose::Batch);
    assert_eq!(file.bytes, 0);
    assert_eq!(file.filename, "");
}

#[rstest]
#[case::spaces(" 42 ")]
#[case::plain("42")]
fn a_size_text_is_parsed_as_an_integer(#[case] size: &str) {
    let body = format!(r#"{{"id":"my-bucket/o/1","size":"{size}",{CREATED_JSON}}}"#);
    let file = decode_vertex_file_metadata(body.as_bytes(), MetadataShape::Upload, &utc()).unwrap();
    assert_eq!(file.bytes, 42);
}

#[test]
fn a_size_that_is_not_an_integer_is_refused() {
    let failure = decode_vertex_file_metadata(
        br#"{"id":"my-bucket/o/1","size":"4.2"}"#,
        MetadataShape::Upload,
        &utc(),
    )
    .expect_err("the source runs int() over the text");
    assert!(matches!(failure, VertexFilesError::Size { .. }));
}

#[test]
fn the_last_slash_component_is_always_a_generation_suffix() {
    let body = format!(r#"{{"id":"my-bucket/report.jsonl",{CREATED_JSON}}}"#);
    let file =
        decode_vertex_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc()).unwrap();
    assert_eq!(file.id, "gs://my-bucket");
}

#[test]
fn a_reply_without_an_id_has_only_the_scheme() {
    let body = format!(r#"{{"name":"x",{CREATED_JSON}}}"#);
    let file = decode_vertex_file_metadata(body.as_bytes(), MetadataShape::Upload, &utc()).unwrap();
    assert_eq!(file.id, "gs://");
}

#[test]
fn a_reply_without_a_creation_time_is_refused() {
    let failure = decode_vertex_file_metadata(
        br#"{"id":"my-bucket/o/1"}"#,
        MetadataShape::Retrieve,
        &utc(),
    )
    .expect_err("the source always parses timeCreated");
    assert!(matches!(failure, VertexFilesError::Timestamp(_)));
}

#[test]
fn an_unknown_purpose_is_refused() {
    let failure = decode_vertex_file_metadata(
        &upload_body(r#","purpose":"chat""#),
        MetadataShape::Upload,
        &utc(),
    )
    .expect_err("the file record's purpose is a closed set");
    assert!(matches!(failure, VertexFilesError::Record(_)));
}

#[test]
fn a_body_that_is_not_a_record_is_refused() {
    let failure = decode_vertex_file_metadata(b"[]", MetadataShape::Upload, &utc())
        .expect_err("an array is not an object record");
    assert!(matches!(failure, VertexFilesError::Record(_)));
}

#[test]
fn bytes_after_the_record_are_refused() {
    let failure = decode_vertex_file_metadata(
        br#"{"id":"my-bucket/o/1"} {}"#,
        MetadataShape::Upload,
        &utc(),
    )
    .expect_err("a second value is not part of the record");
    assert!(matches!(failure, VertexFilesError::Decode(_)));
}

#[test]
fn a_body_at_the_limit_is_still_parsed() {
    let mut body = format!(r#"{{"id":"my-bucket/o/1",{CREATED_JSON}}}"#).into_bytes();
    body.resize(METADATA_BODY_LIMIT, b' ');
    let file = decode_vertex_file_metadata(&body, MetadataShape::Upload, &utc()).unwrap();
    assert_eq!(file.id, "gs://my-bucket/o");
}

#[test]
fn a_body_over_the_limit_is_refused() {
    let body = vec![b' '; METADATA_BODY_LIMIT.saturating_add(1)];
    assert!(matches!(
        decode_vertex_file_metadata(&body, MetadataShape::Retrieve, &utc()),
        Err(VertexFilesError::ResponseTooLarge { size, limit })
            if size == METADATA_BODY_LIMIT.saturating_add(1) && limit == METADATA_BODY_LIMIT
    ));
}

#[test]
fn a_delete_reply_rebuilds_the_object_from_the_request_url() {
    let deleted = deleted_file(Some(
        "https://storage.googleapis.com/storage/v1/b/my-bucket/o/litellm-vertex-files%2Fuploads%2Frows.jsonl",
    ));
    assert_eq!(
        deleted,
        FileDeleted {
            id: format!("gs://my-bucket/{OBJECT_PATH}"),
            object: FileKind::File,
            deleted: true,
        }
    );
}

#[test]
fn a_delete_url_drops_its_query_before_decoding() {
    let deleted = deleted_file(Some(
        "https://storage.googleapis.com/storage/v1/b/my-bucket/o/a%3Fb?alt=json",
    ));
    assert_eq!(deleted.id, "gs://my-bucket/a?b");
}

#[test]
fn a_delete_url_keeps_a_non_utf8_object_as_replacement_characters() {
    let deleted = deleted_file(Some("https://h/b/my-bucket/o/%FF"));
    assert_eq!(deleted.id, "gs://my-bucket/\u{fffd}");
}

#[rstest]
#[case::no_url(None)]
#[case::no_bucket_segment(Some("https://h/storage/v1/o/object"))]
#[case::no_object_segment(Some("https://h/storage/v1/b/my-bucket"))]
fn an_unusable_delete_url_falls_back_to_deleted(#[case] url: Option<&str>) {
    assert_eq!(deleted_file(url).id, "deleted");
}

#[rstest]
fn replays_the_vertex_file_metadata_corpus(
    #[files("fuzz/corpus/vertex_file_metadata/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a readable seed");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a utf-8 seed name");
    check_corpus_seed(name, &bytes);
}
