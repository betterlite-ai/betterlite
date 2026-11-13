use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};
use rstest::rstest;

use crate::api::file_contracts::config::{BaseFilesConfig as _, FileLogging, FileParams};
use crate::manus_files::MANUS_API_BASE;
use crate::manus_files::error::ManusFilesError;
use crate::manus_files::reply::decode_file_list;

use super::support::{NOW, params, plain_config, response};

fn files_of(body: &str) -> Vec<FileObject> {
    decode_file_list(body.as_bytes(), NOW).expect("the reply is a Manus file list")
}

#[rstest]
#[case::present(Some("assistants"), r#"{"purpose":"assistants"}"#)]
#[case::absent(None, "{}")]
#[case::empty(Some(""), "{}")]
fn the_list_request_sends_the_purpose_only_when_present(
    #[case] purpose: Option<&str>,
    #[case] expected: &str,
) {
    let (url, query) = plain_config(NOW)
        .transform_list_files_request(purpose, &FileParams::new(), &FileParams::new())
        .expect("the base resolves");
    assert_eq!(url, format!("{MANUS_API_BASE}/v1/files"));
    assert_eq!(query, params(expected));
}

#[test]
fn the_list_request_uses_the_litellm_params_api_base() {
    let (url, _) = plain_config(NOW)
        .transform_list_files_request(
            None,
            &FileParams::new(),
            &params(r#"{"api_base":"http://host/v1"}"#),
        )
        .expect("the base resolves");
    assert_eq!(url, "http://host/v1/files");
}

#[test]
fn list_items_keep_their_own_status() {
    let body = r#"{"data":[
        {"id":"a","status":"pending"},
        {"id":"b"},
        {"id":"c","status":"error"}
    ]}"#;
    let files = files_of(body);
    let statuses: Vec<Option<FileStatus>> = files.iter().map(|file| file.status).collect();
    assert_eq!(
        statuses,
        vec![
            Some(FileStatus::Pending),
            Some(FileStatus::Uploaded),
            Some(FileStatus::Error)
        ]
    );
}

#[test]
fn list_items_default_the_other_fields() {
    let files = files_of(r#"{"data":[{}]}"#);
    let file = files.first().expect("one item comes back");
    assert_eq!(file.id, "");
    assert_eq!(file.bytes, 0);
    assert_eq!(file.created_at, NOW);
    assert_eq!(file.filename, "");
    assert_eq!(file.object, FileKind::File);
    assert_eq!(file.purpose, FilesPurpose::Assistants);
    assert_eq!(file.status, Some(FileStatus::Uploaded));
    assert_eq!(file.status_details, None);
}

#[test]
fn list_items_keep_their_own_fields() {
    let body = r#"{"data":[{
        "id":"file-1",
        "bytes":4,
        "created_at":"2025-01-02T03:04:05Z",
        "filename":"a.txt",
        "purpose":"vision",
        "status_details":"fine"
    }]}"#;
    let files = files_of(body);
    let file = files.first().expect("one item comes back");
    assert_eq!(file.id, "file-1");
    assert_eq!(file.bytes, 4);
    assert_eq!(file.created_at, 1_735_787_045);
    assert_eq!(file.filename, "a.txt");
    assert_eq!(file.purpose, FilesPurpose::Vision);
    assert_eq!(file.status_details, Some(String::from("fine")));
}

#[test]
fn a_list_item_with_an_unparsable_created_at_falls_back_to_now() {
    let files = files_of(r#"{"data":[{"created_at":"yesterday"}]}"#);
    assert_eq!(files.first().expect("one item comes back").created_at, NOW);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"data":null}"#)]
fn a_list_without_items_is_empty(#[case] body: &str) {
    assert!(files_of(body).is_empty());
}

#[rstest]
#[case::data_not_a_list(r#"{"data":7}"#)]
#[case::unknown_status(r#"{"data":[{"status":"weird"}]}"#)]
#[case::not_json("not json")]
fn a_bad_list_reply_is_refused(#[case] body: &str) {
    let error = decode_file_list(body.as_bytes(), NOW).expect_err("the reply is not a file list");
    assert!(matches!(error, ManusFilesError::ListResponse { .. }));
}

#[test]
fn the_trait_list_response_returns_the_items() {
    let files = plain_config(NOW)
        .transform_list_files_response(
            &response(r#"{"data":[{"id":"file-1"}]}"#),
            &mut FileLogging::default(),
            &params("{}"),
        )
        .expect("the reply is a file list");
    assert_eq!(files.len(), 1_usize);
    assert_eq!(files.first().expect("one item comes back").id, "file-1");
}
