#![cfg(test)]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;

use super::super::error::MistralFilesError;
use super::super::records::{MistralFile, MistralFileDeleted, MistralPurpose};
use super::super::transformation::{
    FileUploadRequest, create_upload, file_url, list_request, openai_purpose, to_file_deleted,
    to_file_object, upload_purpose,
};
use super::settings;
use crate::files::records::{BinaryContent, FileKind, FileStatus, FilesPurpose};

fn file(text: &str) -> MistralFile {
    serde_json::from_str(text).expect("a native file")
}

fn upload(file_data: FileData, purpose: Option<&str>) -> FileUploadRequest {
    FileUploadRequest {
        file: file_data,
        purpose: purpose.map(str::to_owned),
    }
}

#[rstest]
#[case("fine-tune", MistralPurpose::FineTune, "fine-tune")]
#[case("batch", MistralPurpose::Batch, "batch")]
#[case("ocr", MistralPurpose::Ocr, "ocr")]
#[case("user_data", MistralPurpose::Ocr, "ocr")]
fn upload_purposes_map_onto_mistral(
    #[case] purpose: &str,
    #[case] expected: MistralPurpose,
    #[case] wire: &str,
) {
    let mapped = upload_purpose(Some(purpose)).expect("a supported purpose");
    assert_eq!(mapped, expected);
    assert_eq!(mapped.wire_name(), wire);
}

#[test]
fn a_missing_upload_purpose_defaults_to_batch() {
    assert_eq!(
        upload_purpose(None).expect("the default"),
        MistralPurpose::Batch
    );
}

#[rstest]
#[case("assistants")]
#[case("messages")]
#[case("vision")]
fn upload_rejects_purposes_mistral_lacks(#[case] purpose: &str) {
    let error = upload_purpose(Some(purpose)).expect_err("an unsupported purpose");
    assert_eq!(error.status_code(), Some(400));
    let text = error.to_string();
    for supported in ["fine-tune", "batch", "ocr", "user_data"] {
        assert!(text.contains(supported), "{text} names {supported}");
    }
}

#[test]
fn upload_fills_the_filename_and_content_type_defaults() {
    let created = create_upload(
        &upload(FileData::Direct(FileBody::Bytes(vec![1, 2])), Some("batch")),
        1_704_153_600,
        None,
        &settings(None, None),
    )
    .expect("an upload");
    assert_eq!(created.url, "https://api.mistral.ai/v1/files");
    assert_eq!(created.filename, "file_1704153600.jsonl");
    assert_eq!(created.content_type, "application/octet-stream");
    assert_eq!(created.content, vec![1, 2]);
    assert_eq!(created.purpose, MistralPurpose::Batch);
}

#[test]
fn upload_guesses_the_content_type_from_the_filename() {
    let named = FileData::Tuple2 {
        filename: Some(String::from("notes.txt")),
        content: FileBody::Bytes(vec![1]),
    };
    let created = create_upload(
        &upload(named, Some("fine-tune")),
        5,
        None,
        &settings(None, None),
    )
    .expect("an upload");
    assert_eq!(created.filename, "notes.txt");
    assert_eq!(created.content_type, "text/plain");
    assert_eq!(created.purpose, MistralPurpose::FineTune);
}

#[test]
fn upload_keeps_an_explicit_content_type() {
    let typed = FileData::Tuple3 {
        filename: Some(String::from("data.bin")),
        content: FileBody::Bytes(vec![1]),
        content_type: Some(String::from("application/x-custom")),
    };
    let created = create_upload(
        &upload(typed, Some("batch")),
        5,
        None,
        &settings(None, None),
    )
    .expect("an upload");
    assert_eq!(created.content_type, "application/x-custom");
}

#[test]
fn upload_rejects_a_bare_string_body() {
    let error = create_upload(
        &upload(
            FileData::Direct(FileBody::BareString(String::from("x"))),
            Some("batch"),
        ),
        5,
        None,
        &settings(None, None),
    )
    .expect_err("a bare string is rejected");
    assert!(matches!(error, MistralFilesError::Content(_)));
    assert_eq!(error.status_code(), None);
}

#[rstest]
#[case(None, "https://api.mistral.ai/v1/files/file-1", "")]
#[case(None, "https://api.mistral.ai/v1/files/file-1/content", "/content")]
#[case(
    Some("https://proxy.example/v1/"),
    "https://proxy.example/v1/files/file-1",
    ""
)]
fn file_urls_resolve_the_mistral_origin_and_suffix(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
    #[case] suffix: &str,
) {
    let url = file_url(Some("file-1"), suffix, api_base, &settings(None, None)).expect("a URL");
    assert_eq!(url, expected);
}

#[test]
fn file_urls_encode_each_id_as_one_segment() {
    let url = file_url(Some("a b/c"), "", None, &settings(None, None)).expect("a URL");
    assert_eq!(url, "https://api.mistral.ai/v1/files/a%20b%2Fc");
}

#[rstest]
#[case(None)]
#[case(Some(""))]
#[case(Some(".."))]
fn file_urls_reject_an_absent_or_dot_id(#[case] file_id: Option<&str>) {
    let error = file_url(file_id, "", None, &settings(None, None)).expect_err("no URL");
    assert_eq!(error.status_code(), None);
}

#[test]
fn content_requires_a_file_id() {
    let error = file_url(None, "/content", None, &settings(None, None)).expect_err("no URL");
    assert!(matches!(error, MistralFilesError::FileIdRequired));
}

#[test]
fn content_appends_the_content_suffix() {
    let url = file_url(Some("file-1"), "/content", None, &settings(None, None)).expect("a URL");
    assert_eq!(url, "https://api.mistral.ai/v1/files/file-1/content");
}

#[rstest]
#[case(None, None)]
#[case(Some(""), None)]
fn list_sends_no_purpose_for_a_falsey_value(#[case] purpose: Option<&str>, _expected: Option<u8>) {
    let (url, query) = list_request(purpose, None, &settings(None, None)).expect("a request");
    assert_eq!(url, "https://api.mistral.ai/v1/files");
    assert!(query.is_empty());
}

#[rstest]
#[case("batch", "batch")]
#[case("fine-tune", "fine-tune")]
#[case("ocr", "ocr")]
#[case("user_data", "ocr")]
fn list_maps_the_requested_purpose(#[case] purpose: &str, #[case] wire: &str) {
    let (_, query) = list_request(Some(purpose), None, &settings(None, None)).expect("a request");
    assert_eq!(query, vec![(String::from("purpose"), String::from(wire))]);
}

#[test]
fn list_rejects_a_purpose_mistral_lacks() {
    let error = list_request(Some("assistants"), None, &settings(None, None))
        .expect_err("an unsupported purpose");
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case("ocr", FilesPurpose::UserData)]
#[case("user_data", FilesPurpose::UserData)]
#[case("playground", FilesPurpose::UserData)]
#[case("fine-tune", FilesPurpose::FineTune)]
#[case("batch", FilesPurpose::Batch)]
fn readback_maps_onto_openai_purposes(#[case] native: &str, #[case] expected: FilesPurpose) {
    assert_eq!(openai_purpose(native), expected);
}

#[test]
fn readback_fills_the_file_object_defaults() {
    let object = to_file_object(
        &file(r#"{"id": "file_1", "purpose": "ocr"}"#),
        1_704_153_600,
    );
    assert_eq!(object.id, "file_1");
    assert_eq!(object.bytes, 0);
    assert_eq!(object.created_at, 1_704_153_600);
    assert_eq!(object.filename, "");
    assert_eq!(object.object, FileKind::File);
    assert_eq!(object.purpose, FilesPurpose::UserData);
    assert_eq!(object.status, Some(FileStatus::Uploaded));
    assert_eq!(object.expires_at, None);
    assert_eq!(object.status_details, None);
}

#[test]
fn readback_keeps_a_supplied_creation_time_and_expiry() {
    let object = to_file_object(
        &file(r#"{"id": "file_1", "created_at": 7, "expires_at": 9}"#),
        100,
    );
    assert_eq!(object.created_at, 7);
    assert_eq!(object.expires_at, Some(9));
    assert_eq!(object.purpose, FilesPurpose::Batch);
}

#[test]
fn delete_readback_defaults_deleted_to_true() {
    let deleted: MistralFileDeleted = serde_json::from_str(r#"{"id": "file_1"}"#).expect("deleted");
    let object = to_file_deleted(&deleted);
    assert_eq!(object.id, "file_1");
    assert_eq!(object.object, FileKind::File);
    assert!(object.deleted);
}

#[test]
fn delete_readback_keeps_an_explicit_false() {
    let deleted: MistralFileDeleted =
        serde_json::from_str(r#"{"id": "file_1", "deleted": false}"#).expect("deleted");
    assert!(!to_file_deleted(&deleted).deleted);
}

#[test]
fn a_content_reply_keeps_the_upstream_status_headers_and_bytes() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/pdf"),
    );
    let reply = UpstreamResponse {
        status: 206,
        headers,
        body: vec![1, 2, 3],
    };
    let content = BinaryContent::from_response(reply);
    assert_eq!(content.status, 206);
    assert_eq!(content.body, vec![1, 2, 3]);
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("application/pdf")
    );
}
