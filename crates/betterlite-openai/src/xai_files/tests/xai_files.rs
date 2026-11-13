use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;

use crate::api::batch_records::WireDecodeLimits;
use crate::files::records::{FileDeleted, FileObject};
use crate::files::records::{FileKind, FileStatus, FilesPurpose};
use crate::xai_batches::error::XaiBatchesError;
use crate::xai_files::error::XaiFilesError;
use crate::xai_files::{FilesTarget, SUPPORTED_OPENAI_PARAMS, create_upload, files_url, next_page};
use crate::xai_files::{XaiFile, XaiFileDeleted, XaiFileList, decode_xai_upload_reply};
use crate::xai_files::{decode_file_reply, to_file_deleted, to_file_list, to_file_object};
use crate::xai_stt::decode_xai_stt_reply;

fn settings() -> Settings {
    Settings {
        xai_key: None,
        generic_api_key: None,
        environment_key: None,
        api_base: None,
    }
}

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1 << 20,
        max_event_bytes: 1 << 10,
        max_items: 1000,
    }
}

fn response(status: u16, body: &[u8]) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    }
}

fn direct(content: &[u8]) -> FileData {
    FileData::Direct(FileBody::Bytes(content.to_vec()))
}

fn decode_file_object(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    now: i64,
) -> Result<FileObject, XaiFilesError> {
    decode_file_reply(reply, limits, |file: XaiFile| to_file_object(&file, now))
}

fn decode_file_list(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
    now: i64,
) -> Result<Vec<FileObject>, XaiFilesError> {
    decode_file_reply(reply, limits, |page: XaiFileList| to_file_list(&page, now))
}

fn decode_file_deleted(
    reply: &UpstreamResponse,
    limits: &WireDecodeLimits,
) -> Result<FileDeleted, XaiFilesError> {
    decode_file_reply(reply, limits, |deleted: XaiFileDeleted| {
        to_file_deleted(&deleted)
    })
}

#[test]
fn upload_keeps_the_default_filename_mime_and_purpose() {
    let parts = create_upload(Some(&direct(b"body")), None, 1_700_000_000).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "file");
    assert_eq!(parts[0].file_name.as_deref(), Some("file_1700000000.jsonl"));
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(parts[0].data, b"body");
    assert_eq!(parts[1].name, "purpose");
    assert_eq!(parts[1].file_name, None);
    assert_eq!(parts[1].content_type, None);
    assert_eq!(parts[1].data, b"batch");
}

#[test]
fn a_blank_filename_and_a_blank_purpose_fall_back_to_the_defaults() {
    let file = FileData::Tuple3 {
        filename: Some(String::new()),
        content: FileBody::Bytes(b"x".to_vec()),
        content_type: None,
    };
    let parts = create_upload(Some(&file), Some(""), 7).unwrap();
    assert_eq!(parts[0].file_name.as_deref(), Some("file_7.jsonl"));
    assert_eq!(parts[1].data, b"batch");
}

#[test]
fn upload_sends_the_callers_filename_content_type_and_purpose() {
    let file = FileData::Tuple3 {
        filename: Some("a.jsonl".to_owned()),
        content: FileBody::Bytes(b"body".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let parts = create_upload(Some(&file), Some("assistants"), 1).unwrap();
    assert_eq!(parts[0].file_name.as_deref(), Some("a.jsonl"));
    assert_eq!(parts[0].content_type.as_deref(), Some("application/jsonl"));
    assert_eq!(parts[1].data, b"assistants");
}

#[test]
fn a_tuple_without_a_content_type_guesses_it_from_the_filename() {
    let file = FileData::Tuple2 {
        filename: Some("clip.wav".to_owned()),
        content: FileBody::Bytes(b"wav".to_vec()),
    };
    let parts = create_upload(Some(&file), None, 1).unwrap();
    assert_eq!(parts[0].content_type.as_deref(), Some("audio/x-wav"));
}

#[test]
fn a_missing_file_is_rejected_before_anything_is_built() {
    assert!(matches!(
        create_upload(None, None, 1),
        Err(XaiFilesError::FileDataRequired)
    ));
}

#[test]
fn a_bare_string_body_is_rejected() {
    let file = FileData::Direct(FileBody::BareString("/etc/passwd".to_owned()));
    assert!(matches!(
        create_upload(Some(&file), None, 1),
        Err(XaiFilesError::Media(_))
    ));
}

#[test]
fn the_returned_file_reports_batch_and_uploaded() {
    let file = XaiFile {
        id: "file-1".to_owned(),
        bytes: 0,
        created_at: None,
        filename: String::new(),
        purpose: String::new(),
        expires_at: None,
    };
    let object = to_file_object(&file, 10);
    assert_eq!(object.id, "file-1");
    assert_eq!(object.bytes, 0);
    assert_eq!(object.created_at, 10);
    assert_eq!(object.filename, "");
    assert_eq!(object.object, FileKind::File);
    assert_eq!(object.purpose, FilesPurpose::Batch);
    assert_eq!(object.status, Some(FileStatus::Uploaded));
    assert_eq!(object.expires_at, None);
    assert_eq!(object.status_details, None);
}

#[test]
fn an_upstream_purpose_and_status_never_reach_the_returned_file() {
    let body = br#"{"id":"file-2","bytes":5,"created_at":12,"filename":"f","purpose":"assistants","status":"processed","expires_at":9}"#;
    let object = decode_file_object(&response(200, body), &limits(), 99).unwrap();
    assert_eq!(object.created_at, 12);
    assert_eq!(object.bytes, 5);
    assert_eq!(object.filename, "f");
    assert_eq!(object.purpose, FilesPurpose::Batch);
    assert_eq!(object.status, Some(FileStatus::Uploaded));
    assert_eq!(object.expires_at, Some(9));
}

#[rstest]
#[case::missing_id(br#"{"bytes":1}"#)]
#[case::numeric_string_bytes(br#"{"id":"f","bytes":"5"}"#)]
#[case::null_bytes(br#"{"id":"f","bytes":null}"#)]
#[case::null_filename(br#"{"id":"f","filename":null}"#)]
#[case::null_purpose(br#"{"id":"f","purpose":null}"#)]
#[case::not_an_object(br#"["f"]"#)]
fn a_file_reply_of_the_wrong_shape_is_a_decode_error(#[case] body: &[u8]) {
    assert!(matches!(
        decode_xai_upload_reply::<XaiFile>(body, &limits()),
        Err(XaiFilesError::Decode { .. })
    ));
}

#[test]
fn a_null_created_at_takes_the_clock() {
    let object = decode_file_object(
        &response(200, br#"{"id":"f","created_at":null}"#),
        &limits(),
        42,
    )
    .unwrap();
    assert_eq!(object.created_at, 42);
}

#[test]
fn a_list_page_projects_every_file() {
    let page: XaiFileList = decode_xai_upload_reply(
        br#"{"data":[{"id":"a"},{"id":"b","created_at":7,"bytes":2,"filename":"x"}]}"#,
        &limits(),
    )
    .unwrap();
    let files = to_file_list(&page, 5);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].id, "a");
    assert_eq!(files[0].created_at, 5);
    assert_eq!(files[1].created_at, 7);
    assert_eq!(files[1].bytes, 2);
    assert_eq!(files[1].filename, "x");
}

#[test]
fn a_list_page_without_data_is_empty() {
    let files = decode_file_list(&response(200, b"{}"), &limits(), 3).unwrap();
    assert!(files.is_empty());
}

#[rstest]
#[case::null_data(br#"{"data":null}"#)]
#[case::bad_item(br#"{"data":[{"bytes":2}]}"#)]
fn a_list_page_of_the_wrong_shape_is_a_decode_error(#[case] body: &[u8]) {
    assert!(matches!(
        decode_file_list(&response(200, body), &limits(), 3),
        Err(XaiFilesError::Decode { .. })
    ));
}

#[rstest]
#[case::token_and_file(br#"{"data":[{"id":"a"}],"pagination_token":"tok"}"#, true)]
#[case::null_token(br#"{"data":[{"id":"a"}],"pagination_token":null}"#, false)]
#[case::empty_token(br#"{"data":[{"id":"a"}],"pagination_token":""}"#, false)]
#[case::no_token(br#"{"data":[{"id":"a"}]}"#, false)]
#[case::empty_data(br#"{"pagination_token":"tok"}"#, false)]
fn the_cursor_advances_only_with_a_token_and_a_file(#[case] body: &[u8], #[case] advances: bool) {
    let page: XaiFileList = decode_xai_upload_reply(body, &limits()).unwrap();
    let next = next_page(&page, "https://api.x.ai/v1/files");
    assert_eq!(next.is_some(), advances);
    if let Some(page_request) = next {
        assert_eq!(page_request.url, "https://api.x.ai/v1/files");
        assert_eq!(
            page_request.query,
            vec![("pagination_token".to_owned(), "tok".to_owned())]
        );
    }
}

#[test]
fn the_next_page_reads_the_body_without_a_status_check() {
    let reply = response(400, br#"{"data":[{"id":"a"}],"pagination_token":"tok"}"#);
    let page = decode_xai_upload_reply::<XaiFileList>(&reply.body, &limits())
        .map(|page| next_page(&page, "https://api.x.ai/v1/files"))
        .unwrap();
    assert!(page.is_some());
}

#[rstest]
#[case::default(None, "https://api.x.ai/v1/files")]
#[case::trailing_slash(Some("https://proxy.example/v1/"), "https://proxy.example/v1/files")]
#[case::versioned(Some("https://proxy.example/v1"), "https://proxy.example/v1/files")]
#[case::unversioned(Some("https://proxy.example"), "https://proxy.example/v1/files")]
fn the_collection_url_comes_from_the_shared_origin(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        files_url(api_base, FilesTarget::Collection, &settings()).unwrap(),
        expected
    );
}

#[test]
fn a_file_id_is_percent_encoded_in_the_path() {
    assert_eq!(
        files_url(None, FilesTarget::File("a/b c"), &settings()).unwrap(),
        "https://api.x.ai/v1/files/a%2Fb%20c"
    );
    assert_eq!(
        files_url(None, FilesTarget::Content("f1"), &settings()).unwrap(),
        "https://api.x.ai/v1/files/f1/content"
    );
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dotdot("..")]
fn a_file_id_that_is_not_a_path_segment_fails_before_the_call(#[case] file_id: &str) {
    assert!(matches!(
        files_url(None, FilesTarget::File(file_id), &settings()),
        Err(XaiFilesError::Path { .. })
    ));
    assert!(matches!(
        files_url(None, FilesTarget::Content(file_id), &settings()),
        Err(XaiFilesError::Path { .. })
    ));
}

#[test]
fn delete_defaults_to_true_and_keeps_an_explicit_false() {
    let default: XaiFileDeleted = decode_xai_upload_reply(br#"{"id":"f"}"#, &limits()).unwrap();
    assert!(default.deleted);
    let explicit: XaiFileDeleted =
        decode_xai_upload_reply(br#"{"id":"f","deleted":false}"#, &limits()).unwrap();
    assert!(!explicit.deleted);
    let deleted = to_file_deleted(&explicit);
    assert_eq!(deleted.id, "f");
    assert_eq!(deleted.object, FileKind::File);
    assert!(!deleted.deleted);
}

#[test]
fn a_delete_reply_without_an_id_is_a_decode_error() {
    assert!(matches!(
        decode_xai_upload_reply::<XaiFileDeleted>(b"{}", &limits()),
        Err(XaiFilesError::Decode { .. })
    ));
}

#[test]
fn a_delete_reply_keeps_the_projected_object() {
    let deleted = decode_file_deleted(&response(200, br#"{"id":"f"}"#), &limits()).unwrap();
    assert_eq!(deleted.object, FileKind::File);
    assert!(deleted.deleted);
}

#[test]
fn a_non_2xx_file_reply_keeps_its_status() {
    let object = decode_file_object(&response(404, b"nope"), &limits(), 1).unwrap_err();
    assert_eq!(object.status_code(), Some(404));
    let list = decode_file_list(&response(500, b"{}"), &limits(), 1).unwrap_err();
    assert_eq!(list.status_code(), Some(500));
    let deleted = decode_file_deleted(&response(401, b"{}"), &limits()).unwrap_err();
    assert_eq!(deleted.status_code(), Some(401));
}

#[test]
fn a_status_error_keeps_the_headers_the_reply_carried() {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_static("abc"),
    );
    let reply = UpstreamResponse {
        status: 429,
        headers: reply_headers,
        body: b"slow down".to_vec(),
    };
    let error = decode_file_object(&reply, &limits(), 1).unwrap_err();
    match error {
        XaiFilesError::Shared(XaiBatchesError::Status {
            status, headers, ..
        }) => {
            assert_eq!(status, 429);
            assert_eq!(headers.get("x-request-id").map(String::as_str), Some("abc"));
        }
        other @ (XaiFilesError::FileDataRequired
        | XaiFilesError::Media(_)
        | XaiFilesError::Shared(_)
        | XaiFilesError::Path { .. }
        | XaiFilesError::Decode { .. }
        | XaiFilesError::TooLarge { .. }) => {
            panic!("expected a retained status error, got {other:?}")
        }
    }
}

#[test]
fn an_answer_over_the_byte_limit_is_rejected_before_parsing() {
    let limits = WireDecodeLimits {
        max_json_bytes: 4,
        max_event_bytes: 1,
        max_items: 1,
    };
    assert!(matches!(
        decode_xai_upload_reply::<XaiFile>(br#"{"id":"f"}"#, &limits),
        Err(XaiFilesError::TooLarge { limit: 4 })
    ));
}

#[test]
fn a_reply_exactly_at_the_byte_limit_is_decoded() {
    let limits = WireDecodeLimits {
        max_json_bytes: 2,
        max_event_bytes: 1,
        max_items: 1,
    };
    let page: XaiFileList = decode_xai_upload_reply(b"{}", &limits).unwrap();
    assert!(page.data.is_empty());
}

#[test]
fn only_purpose_is_advertised() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["purpose"]);
}

fn decode_tagged(bytes: &[u8]) -> Option<()> {
    let Some((tag, body)) = bytes.split_first() else {
        return Some(());
    };
    let limits = limits();
    match tag {
        b'2' => decode_xai_upload_reply::<XaiFileList>(body, &limits)
            .map(drop)
            .ok(),
        b'3' => decode_xai_upload_reply::<XaiFileDeleted>(body, &limits)
            .map(drop)
            .ok(),
        b'4' => decode_xai_stt_reply(&response(200, body), &limits)
            .map(drop)
            .ok(),
        b'5' => decode_xai_stt_reply(&response(500, body), &limits)
            .map(drop)
            .ok(),
        _ => decode_xai_upload_reply::<XaiFile>(body, &limits)
            .map(drop)
            .ok(),
    }
}

fn replay_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/xai_uploads");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in &paths {
        let bytes = fs::read(path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let decoded = decode_tagged(&bytes);
        if name.starts_with("valid-") {
            assert!(decoded.is_some(), "{name} does not decode");
        } else {
            assert!(decoded.is_none(), "{name} decodes but should not");
        }
    }
    assert!(!paths.is_empty(), "no xai upload corpus seeds");
}

#[test]
fn replays_xai_uploads_corpus() {
    replay_corpus();
}

#[test]
fn replays_the_xai_uploads_corpus() {
    replay_corpus();
}
