use std::collections::BTreeMap;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::files::records::{
    FileDeleted, FileKind, FileObject, FileStatus, FilesPurpose,
};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;

use super::support::{
    Fixture, NOW, TIMEOUT, client, configured, content, delete, list, retrieve, secret, upload,
};
use crate::files::SUPPORTED_OPENAI_PARAMS;
use crate::files::api::response_error;
use crate::files::error::FilesError;
use crate::files::request::{
    FileCall, FilesClient, FilesUrl, ListCall, Upload, create_file_parts, files_url,
    validate_environment,
};
use crate::files::wire::decode_anthropic_file;

const FULL_FILE: &str = r#"{"id":"file-1","type":"file","filename":"doc.pdf","mime_type":"application/pdf","size_bytes":123,"created_at":"2025-01-01T00:00:00Z","downloadable":true}"#;

const LIST_PAGE: &str = r#"{"data":[{"id":"file-1","filename":"a.txt","size_bytes":3,"created_at":"2025-01-01T00:00:00Z","purpose":"messages"},{"id":"file-2"}],"has_more":true,"first_id":"file-1","last_id":"file-2"}"#;

#[derive(Clone, Copy)]
enum Scenario {
    FileUploadMultipart,
    NativeFileObject,
    CreatedAtTimezone,
    FileListPagination,
    FileDeleteShape,
    FileContentBytes,
    FileBetaHeader,
}

#[rstest]
#[case::file_upload_multipart(Scenario::FileUploadMultipart)]
#[case::native_file_object(Scenario::NativeFileObject)]
#[case::created_at_timezone(Scenario::CreatedAtTimezone)]
#[case::file_list_pagination(Scenario::FileListPagination)]
#[case::file_delete_shape(Scenario::FileDeleteShape)]
#[case::file_content_bytes(Scenario::FileContentBytes)]
#[case::file_beta_header(Scenario::FileBetaHeader)]
#[tokio::test]
async fn files_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::FileUploadMultipart => file_upload_multipart().await,
        Scenario::NativeFileObject => native_file_object().await,
        Scenario::CreatedAtTimezone => created_at_timezone(),
        Scenario::FileListPagination => file_list_pagination().await,
        Scenario::FileDeleteShape => file_delete_shape().await,
        Scenario::FileContentBytes => file_content_bytes().await,
        Scenario::FileBetaHeader => file_beta_header(),
    }
}

fn bytes_file() -> FileData {
    FileData::Direct(FileBody::Bytes(b"x".to_vec()))
}

fn file_data(filename: Option<&str>, content_type: Option<&str>, data: &[u8]) -> FileData {
    match (filename, content_type) {
        (Some(name), Some(kind)) => FileData::Tuple3 {
            filename: Some(name.to_owned()),
            content: FileBody::Bytes(data.to_vec()),
            content_type: Some(kind.to_owned()),
        },
        (Some(name), None) => FileData::Tuple2 {
            filename: Some(name.to_owned()),
            content: FileBody::Bytes(data.to_vec()),
        },
        (None, _) => FileData::Direct(FileBody::Bytes(data.to_vec())),
    }
}

async fn file_upload_multipart() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["purpose"]);
    assert_eq!(
        create_file_parts(None, Some("messages"), NOW)
            .unwrap_err()
            .to_string(),
        "File data is required"
    );
    let simple = file_data(Some("a.txt"), Some("text/plain"), b"hello\n");
    let file_parts = create_file_parts(Some(&simple), Some("messages"), NOW).unwrap();
    assert_eq!(file_parts.len(), 2);
    assert_eq!(file_parts[0].name, "file");
    assert_eq!(file_parts[0].file_name.as_deref(), Some("a.txt"));
    assert_eq!(file_parts[0].content_type.as_deref(), Some("text/plain"));
    assert_eq!(file_parts[0].data, b"hello\n");
    assert_eq!(file_parts[1].name, "purpose");
    assert_eq!(file_parts[1].file_name, None);
    assert_eq!(file_parts[1].content_type, None);
    assert_eq!(file_parts[1].data, b"messages");
    guessed_and_blank_parts();
    upload_over_transport(&simple).await;
    failure_is_status_error().await;
}

fn guessed_and_blank_parts() {
    let guessed = file_data(Some("a.unknownext"), None, b"x");
    let guessed_parts = create_file_parts(Some(&guessed), None, NOW).unwrap();
    assert_eq!(
        guessed_parts[0].content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(guessed_parts[1].data, b"messages");

    let named = file_data(Some(""), Some("text/csv"), b"y");
    let named_parts = create_file_parts(Some(&named), Some("batch"), NOW).unwrap();
    assert_eq!(named_parts[0].file_name.as_deref(), Some("file_1700000000"));
    assert_eq!(named_parts[0].content_type.as_deref(), Some("text/csv"));
    assert_eq!(named_parts[1].data, b"batch");

    let untyped = file_data(Some("x.json"), Some(""), b"z");
    let untyped_parts = create_file_parts(Some(&untyped), Some(""), NOW).unwrap();
    assert_eq!(
        untyped_parts[0].content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(untyped_parts[1].data, b"");

    let direct = file_data(None, None, b"z");
    let direct_parts = create_file_parts(Some(&direct), None, NOW).unwrap();
    assert_eq!(
        direct_parts[0].file_name.as_deref(),
        Some("file_1700000000")
    );
    assert_eq!(
        direct_parts[0].content_type.as_deref(),
        Some("application/octet-stream")
    );
}

async fn upload_over_transport(simple: &FileData) {
    let fixture = Fixture::answering(200, FULL_FILE.as_bytes());
    let settings = configured(Some("sk-files"), None, None);
    let headers = HeaderMap::new();
    let files_client = FilesClient {
        api_key: None,
        api_base: None,
        settings,
        timeout: TIMEOUT,
    };
    let object = upload(
        &fixture,
        &files_client,
        &Upload {
            file: Some(simple),
            purpose: Some("messages"),
            boundary: "boundary-fixture".to_owned(),
            headers: &headers,
        },
    )
    .await
    .unwrap();
    assert_eq!(object.id, "file-1");
    let request = fixture.only_request();
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "https://api.anthropic.com/v1/files");
    assert_eq!(request.deadline, TIMEOUT);
    assert_eq!(request.headers["x-api-key"], "sk-files");
    let RequestBody::Multipart { boundary, parts } = request.body else {
        panic!("an Anthropic upload is multipart");
    };
    assert_eq!(boundary, "boundary-fixture");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].file_name.as_deref(), Some("a.txt"));
    assert_eq!(parts[1].data, b"messages");
}

async fn native_file_object() {
    same_projection().await;
    defaults_and_fallbacks().await;
    get_replies_decode_error_bodies().await;
}

async fn failure_is_status_error() {
    let upload_fixture =
        Fixture::answering_with(413, &[("x-request-id", "req-upload")], b"too large");
    let file = bytes_file();
    let upload_error = upload(
        &upload_fixture,
        &client(Some("sk-files"), None),
        &Upload {
            file: Some(&file),
            purpose: None,
            boundary: "b".to_owned(),
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap_err();
    assert_status_error(
        upload_error,
        413,
        "too large",
        ("x-request-id", "req-upload"),
    );
    assert_eq!(upload_fixture.sent(), 1);

    let delete_fixture =
        Fixture::answering_with(404, &[("x-request-id", "req-delete")], b"missing");
    let delete_error = delete(
        &delete_fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap_err();
    assert_status_error(delete_error, 404, "missing", ("x-request-id", "req-delete"));
    assert_eq!(delete_fixture.sent(), 1);
}

fn assert_status_error(error: FilesError, status: u16, message: &str, header: (&str, &str)) {
    let FilesError::Provider(provider) = error else {
        panic!("a non-2xx reply is the provider status error");
    };
    assert_eq!(provider.status_code, status);
    assert_eq!(provider.message, message);
    let headers: BTreeMap<String, String> =
        provider.headers.unwrap_or_default().into_iter().collect();
    assert_eq!(headers.get(header.0).map(String::as_str), Some(header.1));
}

async fn get_replies_decode_error_bodies() {
    let retrieve_fixture = Fixture::answering_with(
        404,
        &[],
        br#"{"type":"error","error":{"type":"not_found_error"}}"#,
    );
    let file = retrieve(
        &retrieve_fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(file.id, "");
    assert_eq!(file.bytes, 0);
    assert_eq!(file.created_at, NOW);

    let list_fixture = Fixture::answering_with(500, &[], b"{\"type\":\"error\"}");
    let files = list(
        &list_fixture,
        &client(Some("sk-files"), None),
        &ListCall {
            purpose: None,
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert!(files.is_empty());
}

async fn same_projection() {
    let headers = HeaderMap::new();
    let files_client = client(Some("sk-files"), None);
    let file = bytes_file();
    let create_fixture = Fixture::answering(200, FULL_FILE.as_bytes());
    let created = upload(
        &create_fixture,
        &files_client,
        &Upload {
            file: Some(&file),
            purpose: None,
            boundary: "b".to_owned(),
            headers: &headers,
        },
    )
    .await
    .unwrap();
    let retrieve_fixture = Fixture::answering(200, FULL_FILE.as_bytes());
    let retrieved = retrieve(
        &retrieve_fixture,
        &files_client,
        &FileCall {
            file_id: "file-1",
            headers: &headers,
        },
    )
    .await
    .unwrap();
    assert_eq!(created, retrieved);
    assert_full_file(&created);
}

fn assert_full_file(created: &FileObject) {
    assert_eq!(created.id, "file-1");
    assert_eq!(created.bytes, 123);
    assert_eq!(created.created_at, 1_735_689_600);
    assert_eq!(created.filename, "doc.pdf");
    assert_eq!(created.object, FileKind::File);
    assert_eq!(created.purpose, FilesPurpose::Messages);
    assert_eq!(created.status, Some(FileStatus::Uploaded));
    assert_eq!(created.status_details, None);
    assert_eq!(created.expires_at, None);
}

async fn defaults_and_fallbacks() {
    assert_eq!(
        project_body(b"{}").await,
        FileObject {
            id: String::new(),
            bytes: 0,
            created_at: NOW,
            filename: String::new(),
            object: FileKind::File,
            purpose: FilesPurpose::Messages,
            status: Some(FileStatus::Uploaded),
            expires_at: None,
            status_details: None,
        }
    );
    assert_eq!(project_body(br#"{"bytes":456}"#).await.bytes, 456);
    assert_eq!(
        project_body(br#"{"size_bytes":0,"bytes":9}"#).await.bytes,
        0
    );
}

async fn project_body(body: &[u8]) -> FileObject {
    let fixture = Fixture::answering(200, body);
    retrieve(
        &fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap()
}

fn created_at_timezone() {
    check_created_at(&[
        (r#""2025-01-01T00:00:00Z""#, 1_735_689_600),
        (r#""2025-01-01T00:00:00+05:30""#, 1_735_689_600),
        (r#""2025-01-01T00:00:00.987Z""#, 1_735_689_600),
        (r#""2025-1-1T0:0:0""#, 1_735_689_600),
        (r#""2025-01-01t00:00:00""#, 1_735_689_600),
        (r#""2025-01-01T00:00:60""#, 1_735_689_660),
        (r#""2025-01-01T00:00:61""#, 1_735_689_661),
        (r#""2025-01-01T00:00:00-05:00""#, 1_735_689_600),
        (r#""2025-01-01T00:00:00.5+05:00""#, 1_735_689_600),
        (r#""2025-01-01T00:00:00 trailing""#, 1_735_689_600),
        (r#""2025-01-01T00:00:000""#, 1_735_689_600),
        (r#""2025-01-01T00:00:0x""#, NOW),
        (r#""9999-12-31T23:59:59""#, 253_402_300_799),
        (r#""2025-02-30T00:00:00""#, NOW),
        (r#""0000-01-01T00:00:00""#, NOW),
        (r#""25-01-01T00:00:00""#, NOW),
        (r#""""#, NOW),
        ("null", NOW),
        (r#""2025-01-01T24:00:00""#, NOW),
        (r#""2025-13-01T00:00:00""#, NOW),
        (r#""2025-01-01T00:00:62""#, NOW),
        (r#""2025-01-01 00:00:00""#, NOW),
    ]);
    let missing = decode_anthropic_file(b"{}", NOW).unwrap();
    assert_eq!(missing.created_at, NOW);
    let trailing = decode_anthropic_file(br#"{"created_at":"2025-01-01T00:00:00 "}"#, NOW).unwrap();
    assert_eq!(trailing.created_at, 1_735_689_600);
}

fn check_created_at(cases: &[(&str, i64)]) {
    for (value, expected) in cases {
        let body = format!(r#"{{"created_at":{value}}}"#);
        let file = decode_anthropic_file(body.as_bytes(), NOW).unwrap();
        assert_eq!(file.created_at, *expected, "{value}");
    }
}

async fn file_list_pagination() {
    list_page().await;
    list_query_shapes().await;
    get_replies_decode_error_bodies().await;
}

async fn list_page() {
    let files_client = client(Some("sk-files"), None);
    let headers = HeaderMap::new();
    let fixture = Fixture::answering(200, LIST_PAGE.as_bytes());
    let files = list(
        &fixture,
        &files_client,
        &ListCall {
            purpose: Some("messages"),
            headers: &headers,
        },
    )
    .await
    .unwrap();
    let request = fixture.only_request();
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.as_str(), "https://api.anthropic.com/v1/files");
    assert_eq!(
        request.query,
        [("purpose".to_owned(), "messages".to_owned())]
    );
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].id, "file-1");
    assert_eq!(files[0].bytes, 3);
    assert_eq!(files[0].created_at, 1_735_689_600);
    assert_eq!(files[0].purpose, FilesPurpose::Messages);
    assert_eq!(files[1].id, "file-2");
    assert_eq!(files[1].created_at, NOW);
    assert_eq!(files[1].purpose, FilesPurpose::Messages);
}

async fn list_query_shapes() {
    let files_client = client(Some("sk-files"), None);
    let headers = HeaderMap::new();
    assert_eq!(
        list_query(Some(""), &files_client, &headers).await,
        Vec::new()
    );
    assert_eq!(list_query(None, &files_client, &headers).await, Vec::new());
    let empty = Fixture::answering(200, br#"{"has_more":false}"#);
    let files = list(
        &empty,
        &files_client,
        &ListCall {
            purpose: None,
            headers: &headers,
        },
    )
    .await
    .unwrap();
    assert!(files.is_empty());
}

async fn list_query(
    purpose: Option<&str>,
    files_client: &FilesClient,
    headers: &HeaderMap,
) -> Vec<(String, String)> {
    let fixture = Fixture::answering(200, br#"{"data":[]}"#);
    list(&fixture, files_client, &ListCall { purpose, headers })
        .await
        .unwrap();
    fixture.only_request().query
}

async fn file_delete_shape() {
    retrieve_encoding().await;
    delete_projection().await;
    check_delete_rejections().await;
    trailing_base().await;
    failure_is_status_error().await;
}

async fn retrieve_encoding() {
    let fixture = Fixture::answering(200, FULL_FILE.as_bytes());
    retrieve(
        &fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "a/b?c#d e",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    let request = fixture.only_request();
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), "/v1/files/a%2Fb%3Fc%23d%20e");
    assert!(request.query.is_empty());
    assert!(matches!(request.body, RequestBody::Empty));
}

async fn delete_projection() {
    delete_object().await;
    delete_defaults().await;
}

async fn delete_object() {
    let files_client = client(Some("sk-files"), None);
    let headers = HeaderMap::new();
    let fixture = Fixture::answering(
        200,
        br#"{"id":"file-1","type":"file","deleted":false,"object":"banana"}"#,
    );
    let deleted = delete(
        &fixture,
        &files_client,
        &FileCall {
            file_id: "file-1",
            headers: &headers,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        deleted,
        FileDeleted {
            id: "file-1".to_owned(),
            object: FileKind::File,
            deleted: true,
        }
    );
    let request = fixture.only_request();
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(request.url.path(), "/v1/files/file-1");
    assert!(request.query.is_empty());
    assert!(matches!(request.body, RequestBody::Empty));
}

async fn delete_defaults() {
    let files_client = client(Some("sk-files"), None);
    let headers = HeaderMap::new();
    let empty = Fixture::answering(200, b"{}");
    let deleted = delete(
        &empty,
        &files_client,
        &FileCall {
            file_id: "file-1",
            headers: &headers,
        },
    )
    .await
    .unwrap();
    assert_eq!(deleted.id, "");
    assert!(deleted.deleted);
}

async fn check_delete_rejections() {
    let files_client = client(Some("sk-files"), None);
    let headers = HeaderMap::new();
    for (file_id, message) in [
        ("", "file_id is required"),
        (".", "file_id cannot be a dot path segment"),
        ("..", "file_id cannot be a dot path segment"),
    ] {
        let fixture = Fixture::answering(200, b"{}");
        let error = delete(
            &fixture,
            &files_client,
            &FileCall {
                file_id,
                headers: &headers,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), message);
        assert_eq!(fixture.sent(), 0);
    }
}

async fn trailing_base() {
    let trailing = client(Some("sk-files"), Some("https://api.test/v1///"));
    let fixture = Fixture::answering(200, b"{}");
    delete(
        &fixture,
        &trailing,
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.only_request().url.as_str(),
        "https://api.test/v1/v1/files/file-1"
    );
}

async fn file_content_bytes() {
    content_result().await;
    content_prefixes().await;
    content_redirect().await;
    content_error().await;
    first_failing_status_is_error().await;
}

async fn content_result() {
    let fixture = Fixture::answering_with(
        200,
        &[("content-type", "application/octet-stream")],
        &[0, 255, 10, 128],
    );
    let content = content(
        &fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(content.status, 200);
    assert_eq!(content.body, [0, 255, 10, 128]);
    assert_eq!(content.headers["content-type"], "application/octet-stream");
    let request = fixture.only_request();
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), "/v1/files/file-1/content");
}

async fn content_prefixes() {
    let batch_fixture = Fixture::answering(200, b"[]");
    content(
        &batch_fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "msgbatch_abc/1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        batch_fixture.only_request().url.path(),
        "/v1/messages/batches/msgbatch_abc%2F1/results"
    );

    let upper_fixture = Fixture::answering(200, b"[]");
    content(
        &upper_fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "MSGBATCH_abc",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        upper_fixture.only_request().url.path(),
        "/v1/files/MSGBATCH_abc/content"
    );
}

async fn content_redirect() {
    let fixture = Fixture::answering(302, b"moved");
    let content = content(
        &fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(content.status, 302);
    assert_eq!(content.body, b"moved");
}

async fn content_error() {
    let fixture = Fixture::answering_with(
        404,
        &[("content-type", "text/plain"), ("x-request-id", "req-err")],
        b"denied",
    );
    let error = content(
        &fixture,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap_err();
    let FilesError::Provider(provider) = error else {
        panic!("a non-2xx content reply is the provider status error");
    };
    assert_eq!(provider.status_code, 404);
    assert_eq!(provider.message, "denied");
    let headers: BTreeMap<String, String> =
        provider.headers.unwrap_or_default().into_iter().collect();
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("req-err")
    );
}

async fn first_failing_status_is_error() {
    let boundary = Fixture::answering_with(400, &[("x-request-id", "req-400")], b"bad request");
    let error = content(
        &boundary,
        &client(Some("sk-files"), None),
        &FileCall {
            file_id: "file-1",
            headers: &HeaderMap::new(),
        },
    )
    .await
    .unwrap_err();
    assert_status_error(error, 400, "bad request", ("x-request-id", "req-400"));
}

fn file_beta_header() {
    complete_urls();
    caller_headers_are_overwritten();
    auth_sources();
    wire_request();
    error_forwarding();
}

fn complete_urls() {
    let settings = configured(None, None, None);
    assert_eq!(
        files_url(None, &settings, FilesUrl::Files).unwrap(),
        "https://api.anthropic.com/v1/files"
    );
    assert_eq!(
        files_url(
            Some("https://api.anthropic.com/v1/files"),
            &settings,
            FilesUrl::Files
        )
        .unwrap(),
        "https://api.anthropic.com/v1/files/v1/files"
    );
    assert_eq!(
        files_url(Some("https://api.test/v1///"), &settings, FilesUrl::Files).unwrap(),
        "https://api.test/v1/v1/files"
    );
    assert_eq!(
        files_url(Some(""), &settings, FilesUrl::Files).unwrap(),
        "https://api.anthropic.com/v1/files"
    );
    let env_settings = configured(None, None, Some("https://env-base.example/"));
    assert_eq!(
        files_url(None, &env_settings, FilesUrl::Files).unwrap(),
        "https://env-base.example/v1/files"
    );
    assert_eq!(
        files_url(None, &env_settings, FilesUrl::File("file-1")).unwrap(),
        "https://env-base.example/v1/files/file-1"
    );
}

fn caller_headers_are_overwritten() {
    let settings = configured(None, None, None);
    let key = secret("sk-files");
    let mut caller = HeaderMap::new();
    caller.insert("anthropic-version", HeaderValue::from_static("old"));
    caller.insert("anthropic-beta", HeaderValue::from_static("caller-beta"));
    caller.insert("x-keep", HeaderValue::from_static("keep"));
    caller.insert("content-type", HeaderValue::from_static("application/json"));
    let mut headers = caller.clone();
    validate_environment(&mut headers, Some(&key), None, None, &settings).unwrap();
    assert_eq!(headers["x-api-key"], "sk-files");
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(headers["anthropic-beta"], "files-api-2025-04-14");
    assert_eq!(headers["x-keep"], "keep");
    assert_eq!(headers["content-type"], "application/json");
    assert!(headers["x-api-key"].is_sensitive());

    let mut bare = HeaderMap::new();
    validate_environment(&mut bare, Some(&key), None, None, &settings).unwrap();
    assert!(!bare.contains_key("content-type"));
}

fn auth_sources() {
    let settings = configured(None, None, None);
    let missing =
        validate_environment(&mut HeaderMap::new(), None, None, None, &settings).unwrap_err();
    assert_eq!(
        missing.to_string(),
        "Anthropic API key is required. Set ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN environment \
         variable or pass api_key parameter."
    );

    let token_settings = configured(None, Some("fixture-token"), None);
    let mut token_headers = HeaderMap::new();
    validate_environment(&mut token_headers, None, None, None, &token_settings).unwrap();
    assert_eq!(token_headers["authorization"], "Bearer fixture-token");

    let oauth_settings = configured(Some("sk-ant-oat-fixture"), None, None);
    let mut oauth_headers = HeaderMap::new();
    validate_environment(&mut oauth_headers, None, None, None, &oauth_settings).unwrap();
    assert_eq!(oauth_headers["authorization"], "Bearer sk-ant-oat-fixture");
    assert!(!oauth_headers.contains_key("x-api-key"));

    let mut base_headers = HeaderMap::new();
    validate_environment(
        &mut base_headers,
        Some(&secret("sk-files")),
        Some("https://custom.example"),
        Some("https://params.example"),
        &settings,
    )
    .unwrap();
    assert_eq!(base_headers["x-api-key"], "sk-files");
}

fn wire_request() {
    let mut caller = HeaderMap::new();
    caller.insert("x-keep", HeaderValue::from_static("keep"));
    let files_client = FilesClient {
        api_key: Some(secret("sk-files")),
        api_base: None,
        settings: configured(None, None, None),
        timeout: TIMEOUT,
    };
    let file = bytes_file();
    let request = files_client
        .create_request(
            &Upload {
                file: Some(&file),
                purpose: None,
                boundary: "b".to_owned(),
                headers: &caller,
            },
            NOW,
        )
        .unwrap();
    assert_eq!(request.headers["x-keep"], "keep");
    assert_eq!(request.headers["anthropic-version"], "2023-06-01");
    assert_eq!(request.headers["anthropic-beta"], "files-api-2025-04-14");
}

fn error_forwarding() {
    let error = response_error(
        429,
        "slow down".to_owned(),
        vec![("X-Request-Id".to_owned(), "r1".to_owned())],
    );
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        Some(vec![("X-Request-Id".to_owned(), "r1".to_owned())])
    );
    assert!(!error.status_code_is_synthesized);
}
