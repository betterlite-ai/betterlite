#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::{UploadData, UploadUrlLocation};
use betterlite_openai::files::records::{FileKind, FileStatus, FilesPurpose};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::StudioFilesError;
use super::metadata::{METADATA_BODY_LIMIT, MetadataShape, decode_studio_file_metadata};
use super::plan::{complete_upload_url, create_file_plan, validate_environment};
use super::urls::{decode_delete_response, delete_target, normalize_file_id, retrieve_target};
use super::{RequestParams, SUPPORTED_OPENAI_PARAMS};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::studio_identity::Headers;

const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";
const UPLOAD_URL: &str = "https://studio.example/upload/v1beta/files";

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(environment: Option<&str>, base: Option<&str>) -> Settings {
    Settings {
        environment_key: environment.map(secret),
        generic_api_key: None,
        api_base: base.map_or_else(|| DEFAULT_BASE.to_owned(), str::to_owned),
    }
}

fn params(api_key: Option<&str>, api_base: Option<&str>) -> RequestParams {
    RequestParams {
        api_key: api_key.map(secret),
        api_base: api_base.map(str::to_owned),
    }
}

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::UTC,
    }
}

fn zone(name: &str) -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::get(name).unwrap(),
    }
}

fn file_data(
    filename: Option<&str>,
    content_type: Option<&str>,
    headers: &[(&str, &str)],
    data: &[u8],
) -> FileData {
    FileData::Tuple4 {
        filename: filename.map(str::to_owned),
        content: FileBody::Bytes(data.to_vec()),
        content_type: content_type.map(str::to_owned),
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn create_body(uri: &str, size: &str, created: &str, name: &str) -> Vec<u8> {
    format!(
        r#"{{"file":{{"name":"files/ignored","uri":"{uri}","sizeBytes":"{size}","createTime":"{created}","displayName":"{name}","mimeType":"application/pdf","state":"ACTIVE","source":"UPLOADED"}}}}"#
    )
    .into_bytes()
}

fn exposed(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

#[test]
fn advertise_no_openai_optional_params() {
    assert!(SUPPORTED_OPENAI_PARAMS.is_empty());
}

#[rstest]
#[case::explicit_wins(Some("explicit-key"), Some("environment-key"), "explicit-key")]
#[case::environment_when_absent(None, Some("environment-key"), "environment-key")]
#[case::empty_explicit_falls_through(Some(""), Some("environment-key"), "environment-key")]
fn validate_environment_sets_the_resolved_key(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let caller = BTreeMap::from([("X-Caller".to_owned(), "kept".to_owned())]);
    let headers = validate_environment(
        &caller,
        explicit.map(secret).as_ref(),
        &settings(environment, None),
    )
    .expect("a key resolves");
    assert_eq!(
        exposed(&headers),
        BTreeMap::from([("X-Caller", "kept"), ("x-goog-api-key", expected)])
    );
}

#[test]
fn validate_environment_overwrites_only_the_api_key_header() {
    let caller = [
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("x-goog-api-key".to_owned(), "stale".to_owned()),
    ]
    .into_iter()
    .collect();
    let headers =
        validate_environment(&caller, None, &settings(Some("env-key"), None)).expect("a key");
    assert_eq!(
        exposed(&headers),
        BTreeMap::from([
            ("Content-Type", "text/plain"),
            ("x-goog-api-key", "env-key")
        ])
    );
}

#[rstest]
#[case::nothing_anywhere(None, None)]
#[case::empty_environment(None, Some(""))]
fn validate_environment_requires_a_key(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
) {
    let caller = BTreeMap::new();
    assert!(matches!(
        validate_environment(
            &caller,
            explicit.map(secret).as_ref(),
            &settings(environment, None)
        ),
        Err(StudioFilesError::MissingSettingsKey)
    ));
}

#[rstest]
#[case::default_base(
    None,
    None,
    "https://generativelanguage.googleapis.com/upload/v1beta/files"
)]
#[case::explicit_base(
    Some("https://studio.example"),
    None,
    "https://studio.example/upload/v1beta/files"
)]
#[case::trailing_slash_is_kept(
    Some("https://studio.example/"),
    None,
    "https://studio.example//upload/v1beta/files"
)]
#[case::environment_base(
    None,
    Some("https://env.example"),
    "https://env.example/upload/v1beta/files"
)]
fn upload_url_uses_the_configured_base(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    let settings = settings(Some("env-key"), environment_base);
    let url = complete_upload_url(api_base, None, &params(None, None), &settings)
        .expect("a key resolves");
    assert_eq!(url, expected);
}

#[test]
fn upload_url_accepts_a_litellm_params_key() {
    let url = complete_upload_url(
        None,
        None,
        &params(Some("params-key"), None),
        &settings(None, None),
    )
    .expect("the params key counts");
    assert_eq!(
        url,
        "https://generativelanguage.googleapis.com/upload/v1beta/files"
    );
}

#[test]
fn upload_url_prefers_the_explicit_key_and_requires_one() {
    let with_environment = settings(Some("env-key"), None);
    complete_upload_url(
        None,
        Some(&secret("explicit")),
        &params(Some("params"), None),
        &with_environment,
    )
    .expect("the explicit key wins");
    let empty = params(Some(""), None);
    assert!(matches!(
        complete_upload_url(None, Some(&secret("")), &empty, &settings(None, None)),
        Err(StudioFilesError::MissingApiKey)
    ));
}

#[test]
fn create_plan_requires_file_data() {
    assert!(matches!(
        create_file_plan(None, UPLOAD_URL, 7),
        Err(StudioFilesError::FileDataRequired)
    ));
}

#[test]
fn create_plan_rejects_a_bare_string_body() {
    let file = FileData::Direct(FileBody::BareString("path/on/disk".to_owned()));
    assert!(matches!(
        create_file_plan(Some(&file), UPLOAD_URL, 7),
        Err(StudioFilesError::Media(_))
    ));
}

#[test]
fn create_plan_sends_start_headers_and_metadata() {
    let file = file_data(Some("notes.txt"), None, &[], b"hi");
    let plan = create_file_plan(Some(&file), UPLOAD_URL, 7).expect("the plan builds");
    assert_eq!(plan.initial_request.method, "POST");
    assert_eq!(plan.initial_request.url, UPLOAD_URL);
    assert_eq!(
        plan.initial_request.headers,
        BTreeMap::from([
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("X-Goog-Upload-Command".to_owned(), "start".to_owned()),
            (
                "X-Goog-Upload-Header-Content-Length".to_owned(),
                "2".to_owned()
            ),
            (
                "X-Goog-Upload-Header-Content-Type".to_owned(),
                "text/plain".to_owned()
            ),
            ("X-Goog-Upload-Protocol".to_owned(), "resumable".to_owned()),
        ])
    );
    let UploadData::Text(text) = &plan.initial_request.data else {
        panic!("the start request carries JSON text");
    };
    assert_eq!(text, r#"{"file":{"display_name":"notes.txt"}}"#);
    assert_eq!(plan.upload_url_location, UploadUrlLocation::Headers);
    assert_eq!(plan.upload_url_key.as_deref(), Some("X-Goog-Upload-URL"));
}

#[test]
fn create_plan_sends_the_upload_bytes() {
    let file = file_data(Some("notes.txt"), None, &[], b"hi");
    let plan = create_file_plan(Some(&file), UPLOAD_URL, 7).expect("the plan builds");
    assert_eq!(
        plan.upload_request.headers,
        BTreeMap::from([
            ("Content-Length".to_owned(), "2".to_owned()),
            (
                "X-Goog-Upload-Command".to_owned(),
                "upload, finalize".to_owned()
            ),
            ("X-Goog-Upload-Offset".to_owned(), "0".to_owned()),
        ])
    );
    let UploadData::Bytes(bytes) = &plan.upload_request.data else {
        panic!("the upload request carries bytes");
    };
    assert_eq!(bytes, b"hi");
}

#[test]
fn create_plan_lets_extracted_headers_override_the_start_headers() {
    let file = file_data(
        Some("notes.txt"),
        Some("text/plain"),
        &[
            ("X-Goog-Upload-Command", "custom"),
            ("Content-Type", "application/custom"),
            ("X-Goog-Upload-Offset", "9"),
            ("X-Extra", "kept"),
        ],
        b"hi",
    );
    let plan = create_file_plan(Some(&file), UPLOAD_URL, 7).expect("the plan builds");
    assert_eq!(
        plan.initial_request.headers["X-Goog-Upload-Command"],
        "custom"
    );
    assert_eq!(
        plan.initial_request.headers["Content-Type"],
        "application/custom"
    );
    assert_eq!(plan.initial_request.headers["X-Extra"], "kept");
    assert_eq!(
        plan.initial_request.headers["X-Goog-Upload-Header-Content-Type"],
        "text/plain"
    );
    assert_eq!(plan.upload_request.headers["X-Goog-Upload-Offset"], "0");
    assert_eq!(
        plan.upload_request.headers["X-Goog-Upload-Command"],
        "upload, finalize"
    );
    assert!(!plan.upload_request.headers.contains_key("X-Extra"));
}

#[rstest]
#[case::explicit(Some("application/pdf"), Some("report.bin"), "application/pdf")]
#[case::guessed_from_filename(None, Some("notes.txt"), "text/plain")]
#[case::no_filename(None, None, "application/octet-stream")]
#[case::empty_content_type_is_unset(Some(""), Some("notes.txt"), "text/plain")]
fn create_plan_resolves_the_content_type(
    #[case] content_type: Option<&str>,
    #[case] filename: Option<&str>,
    #[case] expected: &str,
) {
    let file = file_data(filename, content_type, &[], b"hi");
    let plan = create_file_plan(Some(&file), UPLOAD_URL, 7).expect("the plan builds");
    assert_eq!(
        plan.initial_request.headers["X-Goog-Upload-Header-Content-Type"],
        expected
    );
}

#[rstest]
#[case::filename(Some("notes.txt"), "notes.txt")]
#[case::injected_clock(None, "1700000000")]
#[case::empty_filename_is_used_as_given(Some(""), "")]
fn create_plan_names_the_file(#[case] filename: Option<&str>, #[case] expected: &str) {
    let file = file_data(filename, Some("text/plain"), &[], b"hi");
    let plan = create_file_plan(Some(&file), UPLOAD_URL, 1_700_000_000).expect("the plan builds");
    let UploadData::Text(text) = &plan.initial_request.data else {
        panic!("the start request carries JSON text");
    };
    let parsed: Json = serde_json::from_str(text).expect("the plan body is JSON");
    let expected_file = Json::Object(IndexMap::from([(
        "display_name".to_owned(),
        Json::Text(expected.to_owned()),
    )]));
    assert_eq!(
        parsed,
        Json::Object(IndexMap::from([("file".to_owned(), expected_file)]))
    );
}

#[rstest]
#[case::plain("abc123", "files/abc123")]
#[case::prefix("files/abc123", "files/abc123")]
#[case::boundary_slashes("/files/abc123/", "files/abc123")]
#[case::extra_slashes("//abc//", "files/abc")]
#[case::one_prefix_is_removed("files/files/abc", "files/files%2Fabc")]
#[case::nested_id("a/b c", "files/a%2Fb%20c")]
#[case::full_url(
    "https://generativelanguage.googleapis.com/v1beta/files/abc123",
    "files/abc123"
)]
#[case::foreign_host("https://foreign.example/v1beta/files/nested/id", "files/nested%2Fid")]
#[case::first_files_occurrence("https://h/v1beta/xfiles/abc", "files/abc")]
#[case::query_is_dropped("https://h/v1beta/files/abc?alt=json", "files/abc")]
#[case::fragment_is_dropped("https://h/v1beta/files/abc#frag", "files/abc")]
#[case::params_are_dropped("https://h/v1beta/files/abc;v=1", "files/abc")]
#[case::already_encoded("https://h/v1beta/files/a%20b", "files/a%2520b")]
#[case::plus_is_encoded("https://h/v1beta/files/a+b", "files/a%2Bb")]
#[case::uppercase_scheme_is_not_a_url("HTTP://h/files/abc", "files/HTTP%3A%2F%2Fh%2Ffiles%2Fabc")]
#[case::non_ascii("\u{e9}", "files/%C3%A9")]
fn normalize_file_id_encodes_one_segment(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(normalize_file_id(input).expect("the id encodes"), expected);
}

#[rstest]
#[case::empty("")]
#[case::slash("/")]
#[case::host_only("https://h")]
#[case::host_with_slash("https://h/")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[case::url_with_dot_segment("https://h/files/../x")]
fn normalize_file_id_rejects_unusable_ids(#[case] input: &str) {
    assert!(normalize_file_id(input).is_err(), "{input}");
}

#[test]
fn retrieve_target_trims_the_base_and_ignores_the_id_host() {
    let target = retrieve_target(
        "https://foreign.example/v1beta/files/abc123",
        &params(None, Some("https://studio.example///")),
        &settings(Some("env-key"), None),
    )
    .expect("the target builds");
    assert_eq!(target.url, "https://studio.example/v1beta/files/abc123");
    assert!(target.query.is_empty());
}

#[test]
fn retrieve_target_uses_the_default_base_and_params_key() {
    let target = retrieve_target(
        "files/abc",
        &params(Some("params-key"), None),
        &settings(None, None),
    )
    .expect("the target builds");
    assert_eq!(
        target.url,
        "https://generativelanguage.googleapis.com/v1beta/files/abc"
    );
    assert!(target.query.is_empty());
}

#[test]
fn retrieve_target_requires_a_key() {
    assert!(matches!(
        retrieve_target("files/abc", &params(None, None), &settings(Some(""), None)),
        Err(StudioFilesError::MissingApiKey)
    ));
}

#[test]
fn delete_target_keeps_the_base_as_the_helper_returns_it() {
    let target = delete_target(
        "/files/abc/",
        &params(Some("params-key"), Some("https://studio.example///")),
        &settings(None, None),
    )
    .expect("the target builds");
    assert_eq!(target.url, "https://studio.example////v1beta/files/abc");
    assert!(target.query.is_empty());
}

#[test]
fn delete_target_requires_a_key() {
    assert!(matches!(
        delete_target("files/abc", &params(None, None), &settings(None, None)),
        Err(StudioFilesError::MissingApiKey)
    ));
}

#[test]
fn create_reply_takes_the_uri_as_the_file_id() {
    let body = create_body(
        "https://generativelanguage.googleapis.com/v1beta/files/abc",
        "12",
        "2024-01-02T03:04:05.123456Z",
        "report.pdf",
    );
    let file = decode_studio_file_metadata(&body, MetadataShape::Create, &utc())
        .expect("the reply decodes");
    assert_eq!(
        file.id,
        "https://generativelanguage.googleapis.com/v1beta/files/abc"
    );
    assert_eq!(file.bytes, 12);
    assert_eq!(file.created_at, 1_704_164_645);
    assert_eq!(file.filename, "report.pdf");
    assert_eq!(file.object, FileKind::File);
    assert_eq!(file.purpose, FilesPurpose::UserData);
    assert_eq!(file.status, Some(FileStatus::Uploaded));
    assert_eq!(file.status_details, None);
    assert_eq!(file.expires_at, None);
}

#[rstest]
#[case::zulu("2024-01-02T03:04:05.1Z", 1_704_164_645)]
#[case::colon_offset("2024-01-02T03:04:05.1+05:30", 1_704_164_645)]
#[case::compact_offset("2024-01-02T03:04:05.1-0800", 1_704_164_645)]
#[case::offset_with_seconds("2024-01-02T03:04:05.1+05:30:15", 1_704_164_645)]
#[case::offset_with_fraction("2024-01-02T03:04:05.1+05:30:15.5", 1_704_164_645)]
#[case::single_digit_fields("2024-1-2T3:4:5.1+00:00", 1_704_164_645)]
#[case::lowercase_separator("2024-01-02t03:04:05.1+00:00", 1_704_164_645)]
#[case::fraction_truncates("2024-01-02T03:04:05.999999+00:00", 1_704_164_645)]
#[case::before_the_epoch("1969-12-31T23:59:59.5+00:00", -1)]
fn create_reply_parses_fractional_timestamps(#[case] created: &str, #[case] expected: i64) {
    let body = create_body("uri", "1", created, "name");
    let file = decode_studio_file_metadata(&body, MetadataShape::Create, &utc())
        .expect("the reply decodes");
    assert_eq!(file.created_at, expected);
}

#[rstest]
#[case::missing_fraction("2024-01-02T03:04:05Z")]
#[case::seven_fraction_digits("2024-01-02T03:04:05.1234567Z")]
#[case::missing_offset("2024-01-02T03:04:05.1")]
#[case::two_digit_offset("2024-01-02T03:04:05.1+05")]
#[case::lowercase_zulu("2024-01-02T03:04:05.1z")]
#[case::bad_minute("2024-01-02T03:04:05.1+05:60")]
#[case::date_without_time("2024-01-02")]
#[case::impossible_date("2024-02-30T03:04:05.1Z")]
fn create_reply_rejects_malformed_timestamps(#[case] created: &str) {
    let body = create_body("uri", "1", created, "name");
    decode_studio_file_metadata(&body, MetadataShape::Create, &utc())
        .expect_err("the timestamp is malformed");
}

#[test]
fn create_reply_converts_in_the_injected_local_zone() {
    let summer_body = create_body("uri", "1", "2024-07-01T12:00:00.5+00:00", "name");
    let summer =
        decode_studio_file_metadata(&summer_body, MetadataShape::Create, &zone("Europe/Berlin"))
            .expect("the reply decodes");
    let winter_body = create_body("uri", "1", "2024-01-02T03:04:05.5+00:00", "name");
    let winter =
        decode_studio_file_metadata(&winter_body, MetadataShape::Create, &zone("Europe/Berlin"))
            .expect("the reply decodes");
    assert_eq!(summer.created_at, 1_719_828_000);
    assert_eq!(winter.created_at, 1_704_161_045);
}

#[rstest]
#[case::missing_uri(
    r#"{"file":{"sizeBytes":"1","createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}}"#
)]
#[case::missing_size(
    r#"{"file":{"uri":"u","createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}}"#
)]
#[case::missing_create_time(r#"{"file":{"uri":"u","sizeBytes":"1","displayName":"n"}}"#)]
#[case::missing_display_name(
    r#"{"file":{"uri":"u","sizeBytes":"1","createTime":"2024-01-02T03:04:05.1Z"}}"#
)]
#[case::null_uri(r#"{"file":{"uri":null,"sizeBytes":"1","createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}}"#)]
#[case::numeric_size_bytes(
    r#"{"file":{"uri":"u","sizeBytes":1,"createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}}"#
)]
#[case::textual_size_bytes(r#"{"file":{"uri":"u","sizeBytes":"one","createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}}"#)]
#[case::bare_record(
    r#"{"uri":"u","sizeBytes":"1","createTime":"2024-01-02T03:04:05.1Z","displayName":"n"}"#
)]
#[case::not_json("not json")]
fn create_reply_requires_its_fields(#[case] body: &str) {
    decode_studio_file_metadata(body.as_bytes(), MetadataShape::Create, &utc())
        .expect_err("the reply misses a required field");
}

#[rstest]
#[case::padded(" 12 ", 12)]
#[case::signed("+12", 12)]
#[case::negative("-12", -12)]
#[case::leading_zeros("0012", 12)]
fn create_reply_reads_integer_text(#[case] size: &str, #[case] expected: i64) {
    let body = create_body("uri", size, "2024-01-02T03:04:05.1Z", "name");
    let file = decode_studio_file_metadata(&body, MetadataShape::Create, &utc())
        .expect("the reply decodes");
    assert_eq!(file.bytes, expected);
}

#[rstest]
#[case::active("ACTIVE", FileStatus::Processed, None)]
#[case::failed("FAILED", FileStatus::Error, Some(""))]
#[case::processing("PROCESSING", FileStatus::Uploaded, None)]
#[case::unspecified("STATE_UNSPECIFIED", FileStatus::Uploaded, None)]
#[case::unknown("SOMETHING_ELSE", FileStatus::Uploaded, None)]
fn retrieve_reply_maps_the_state(
    #[case] state: &str,
    #[case] expected: FileStatus,
    #[case] details: Option<&str>,
) {
    let body = format!(
        r#"{{"uri":"files/abc","sizeBytes":"5","createTime":"2024-01-02T03:04:05.1Z","displayName":"notes.txt","state":"{state}"}}"#
    );
    let file = decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect("the reply decodes");
    assert_eq!(file.id, "files/abc");
    assert_eq!(file.bytes, 5);
    assert_eq!(file.created_at, 1_704_164_645);
    assert_eq!(file.filename, "notes.txt");
    assert_eq!(file.status, Some(expected));
    assert_eq!(file.status_details.as_deref(), details);
}

#[test]
fn retrieve_reply_defaults_the_optional_fields() {
    let body = r#"{"createTime":"2024-01-02T03:04:05.1Z"}"#;
    let file = decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect("the reply decodes");
    assert_eq!(file.id, "");
    assert_eq!(file.bytes, 0);
    assert_eq!(file.filename, "");
    assert_eq!(file.status, Some(FileStatus::Uploaded));
    assert_eq!(file.status_details, None);
}

#[test]
fn retrieve_reply_treats_null_defaults_as_absent() {
    let body = r#"{"uri":null,"sizeBytes":null,"createTime":"2024-01-02T03:04:05.1Z","displayName":null,"state":null,"error":null}"#;
    let file = decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect("the reply decodes");
    assert_eq!(file.id, "");
    assert_eq!(file.bytes, 0);
    assert_eq!(file.filename, "");
    assert_eq!(file.status, Some(FileStatus::Uploaded));
    assert_eq!(file.status_details, None);
}

#[test]
fn retrieve_reply_requires_the_create_time() {
    let body = r#"{"uri":"files/abc","state":"ACTIVE"}"#;
    decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect_err("the reply has no createTime");
}

#[rstest]
#[case::object(
    r#"{"message": "it's bad", "code": 400}"#,
    r#"{"message":"it's bad","code":400}"#
)]
#[case::text(r#""boom""#, r#""boom""#)]
#[case::number("400", "400")]
#[case::float("1.5", "1.5")]
#[case::boolean("true", "true")]
#[case::list(r#"[1, "a", true, null]"#, r#"[1,"a",true,null]"#)]
#[case::escapes(r#""a\\b\nc\td\"e\u0001f""#, r#""a\\b\nc\td\"e\u0001f""#)]
fn retrieve_reply_renders_failed_details_as_json(#[case] error: &str, #[case] expected: &str) {
    let body =
        format!(r#"{{"createTime":"2024-01-02T03:04:05.1Z","state":"FAILED","error":{error}}}"#);
    let file = decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect("the reply decodes");
    assert_eq!(file.status, Some(FileStatus::Error));
    assert_eq!(file.status_details.as_deref(), Some(expected));
}

#[test]
fn retrieve_reply_ignores_details_for_other_states() {
    let body = r#"{"createTime":"2024-01-02T03:04:05.1Z","state":"ACTIVE","error":{"code":400}}"#;
    let file = decode_studio_file_metadata(body.as_bytes(), MetadataShape::Retrieve, &utc())
        .expect("the reply decodes");
    assert_eq!(file.status_details, None);
}

#[test]
fn metadata_replies_are_bounded() {
    let prefix = r#"{"file":{"uri":"u","sizeBytes":"1","createTime":"2024-01-02T03:04:05.1Z","displayName":""#;
    let suffix = r#""}}"#;
    let build = |padding: usize| -> Vec<u8> {
        format!("{prefix}{}{suffix}", "a".repeat(padding)).into_bytes()
    };
    let smallest = build(0).len();
    let padding = METADATA_BODY_LIMIT
        .checked_sub(smallest)
        .expect("the smallest reply fits the limit");
    let exact = build(padding);
    assert_eq!(exact.len(), METADATA_BODY_LIMIT);
    decode_studio_file_metadata(&exact, MetadataShape::Create, &utc())
        .expect("a reply at the limit decodes");
    let over_padding = padding.checked_add(1).expect("one byte past the limit");
    let over = build(over_padding);
    assert!(matches!(
        decode_studio_file_metadata(&over, MetadataShape::Create, &utc()),
        Err(StudioFilesError::ResponseTooLarge { size, limit })
            if size == METADATA_BODY_LIMIT.saturating_add(1) && limit == METADATA_BODY_LIMIT
    ));
    assert!(matches!(
        decode_studio_file_metadata(&over, MetadataShape::Retrieve, &utc()),
        Err(StudioFilesError::ResponseTooLarge { .. })
    ));
}

#[rstest]
#[case::query_stripped(Some("https://h/v1beta/files/abc?alt=json"), "files/abc")]
#[case::prefixed_segment(Some("https://h/v1beta/files/files/abc"), "files/abc")]
#[case::empty_segment(Some("https://h/v1beta/files/"), "files/")]
#[case::no_files_segment(Some("https://h/v1beta/other/abc"), "deleted")]
#[case::no_request_url(None, "deleted")]
fn delete_reply_reads_the_id_from_the_request_url(
    #[case] url: Option<&str>,
    #[case] expected: &str,
) {
    let deleted = decode_delete_response(200, url, b"{}").expect("the delete succeeds");
    assert_eq!(deleted.id, expected);
    assert_eq!(deleted.object, FileKind::File);
    assert!(deleted.deleted);
}

#[rstest]
#[case::no_content(204)]
#[case::server_error(500)]
fn delete_reply_accepts_only_status_200(#[case] status: u16) {
    assert!(matches!(
        decode_delete_response(status, Some("https://h/v1beta/files/abc"), b"body"),
        Err(StudioFilesError::DeleteFailed { status: found, body }) if found == status && body == "body"
    ));
}

#[test]
fn delete_reply_reports_a_non_utf8_body_by_size() {
    let failure = decode_delete_response(400, None, &[0xff, 0xfe]);
    let message = failure.expect_err("a 400 is a failure").to_string();
    assert!(message.contains("2 bytes that are not UTF-8"), "{message}");
}

fn check_corpus_seed(name: &str, bytes: &[u8]) {
    if name.starts_with("valid-create") {
        decode_studio_file_metadata(bytes, MetadataShape::Create, &utc())
            .expect("this seed is a create reply");
    }
    if name.starts_with("valid-retrieve") {
        decode_studio_file_metadata(bytes, MetadataShape::Retrieve, &utc())
            .expect("this seed is a retrieve reply");
    }
    if name.starts_with("invalid") {
        let create = decode_studio_file_metadata(bytes, MetadataShape::Create, &utc());
        let retrieve = decode_studio_file_metadata(bytes, MetadataShape::Retrieve, &utc());
        let rejected = create.err().or_else(|| retrieve.err());
        assert!(rejected.is_some(), "{name} is not a metadata reply");
    }
    if let Ok(text) = str::from_utf8(bytes) {
        if name.starts_with("valid-id") {
            normalize_file_id(text).expect("this seed is a file id");
        }
        if name.starts_with("invalid-id") {
            normalize_file_id(text).expect_err("this seed is not a file id");
        }
    }
}

#[rstest]
fn replays_the_studio_file_metadata_corpus(
    #[files("fuzz/corpus/studio_file_metadata/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a readable seed");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a utf-8 seed name");
    check_corpus_seed(name, &bytes);
}
