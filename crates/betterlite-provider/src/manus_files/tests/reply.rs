use betterlite_openai::files::records::{FileKind, FileObject, FileStatus, FilesPurpose};
use rstest::rstest;

use crate::api::file_contracts::config::{BaseFilesConfig as _, FileLogging, FileParams};
use crate::api::file_contracts::error::FileContractError;
use crate::manus_files::error::ManusFilesError;
use crate::manus_files::reply::{create_file_object, decode_create_file};

use super::support::{NOW, params, plain_config, response};

fn file_of(body: &str) -> FileObject {
    decode_create_file(body.as_bytes(), NOW).expect("the reply is a Manus file")
}

fn expected_file() -> FileObject {
    FileObject {
        id: String::new(),
        bytes: 0,
        created_at: NOW,
        filename: String::new(),
        object: FileKind::File,
        purpose: FilesPurpose::Assistants,
        status: Some(FileStatus::Uploaded),
        expires_at: None,
        status_details: None,
    }
}

#[test]
fn a_reply_without_fields_takes_every_default() {
    assert_eq!(file_of("{}"), expected_file());
}

#[test]
fn a_reply_with_null_fields_takes_every_default() {
    let body = r#"{
        "id": null,
        "bytes": null,
        "created_at": null,
        "filename": null,
        "purpose": null,
        "status_details": null
    }"#;
    assert_eq!(file_of(body), expected_file());
}

#[test]
fn the_provider_record_is_projected_with_the_fixed_status() {
    let body = r#"{
        "id": "file-1",
        "bytes": 12,
        "created_at": "2025-01-02T03:04:05Z",
        "filename": "doc.pdf",
        "purpose": "batch",
        "status": "pending",
        "status_details": "still going"
    }"#;
    assert_eq!(
        file_of(body),
        FileObject {
            id: String::from("file-1"),
            bytes: 12,
            created_at: 1_735_787_045,
            filename: String::from("doc.pdf"),
            object: FileKind::File,
            purpose: FilesPurpose::Batch,
            status: Some(FileStatus::Uploaded),
            expires_at: None,
            status_details: Some(String::from("still going")),
        }
    );
}

#[test]
fn the_stored_first_reply_wins_over_the_put_body() {
    let litellm_params =
        params(r#"{"initial_file_response":{"id":"stored","filename":"s.txt","bytes":3}}"#);
    let file = create_file_object(
        &response(r#"{"id":"raw","filename":"r.txt","bytes":9}"#),
        &litellm_params,
        NOW,
    )
    .expect("the stored reply is a Manus file");
    assert_eq!(file.id, "stored");
    assert_eq!(file.filename, "s.txt");
    assert_eq!(file.bytes, 3);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"initial_file_response":null}"#)]
fn the_put_body_is_read_without_a_stored_first_reply(#[case] litellm_params: &str) {
    let file = create_file_object(
        &response(r#"{"id":"raw","filename":"r.txt"}"#),
        &params(litellm_params),
        NOW,
    )
    .expect("the raw body is a Manus file");
    assert_eq!(file.id, "raw");
    assert_eq!(file.filename, "r.txt");
}

#[rstest]
#[case::zulu(r#"{"created_at":"2025-01-02T03:04:05Z"}"#, 1_735_787_045)]
#[case::fraction(r#"{"created_at":"2025-01-02T03:04:05.123Z"}"#, 1_735_787_045)]
#[case::offset(r#"{"created_at":"2025-01-02T03:04:05+02:00"}"#, 1_735_787_045)]
#[case::single_digits(r#"{"created_at":"2025-1-2T3:4:5"}"#, 1_735_787_045)]
#[case::lowercase_separator(r#"{"created_at":"2025-01-02t03:04:05"}"#, 1_735_787_045)]
#[case::spaced_day(r#"{"created_at":"2025-01- 2T03:04:05"}"#, 1_735_787_045)]
#[case::trailing_data(r#"{"created_at":"2025-01-02T03:04:05\n"}"#, 1_735_787_045)]
#[case::leap_second(r#"{"created_at":"2025-01-02T23:59:60"}"#, 1_735_862_400)]
#[case::second_61(r#"{"created_at":"2025-01-02T03:04:61"}"#, 1_735_787_101)]
#[case::empty(r#"{"created_at":""}"#, NOW)]
#[case::unparsable(r#"{"created_at":"not-a-time"}"#, NOW)]
#[case::bad_day(r#"{"created_at":"2025-02-30T00:00:00"}"#, NOW)]
#[case::bad_month(r#"{"created_at":"2025-13-01T00:00:00"}"#, NOW)]
#[case::bad_hour(r#"{"created_at":"2025-01-02T24:00:00"}"#, NOW)]
#[case::bad_minute(r#"{"created_at":"2025-01-02T03:60:00"}"#, NOW)]
#[case::bad_second(r#"{"created_at":"2025-01-02T03:04:62"}"#, NOW)]
#[case::year_zero(r#"{"created_at":"0000-01-01T00:00:00"}"#, NOW)]
#[case::absent("{}", NOW)]
#[case::null(r#"{"created_at":null}"#, NOW)]
fn created_at_is_cut_at_nineteen_characters_and_falls_back_to_now(
    #[case] body: &str,
    #[case] expected: i64,
) {
    assert_eq!(file_of(body).created_at, expected);
}

#[test]
fn a_create_reply_ignores_an_unknown_status() {
    let file = file_of(r#"{"status":"something-new"}"#);
    assert_eq!(file.status, Some(FileStatus::Uploaded));
}

#[rstest]
#[case::unknown_purpose(r#"{"purpose":"nope"}"#)]
#[case::wrong_id_type(r#"{"id":7}"#)]
#[case::wrong_bytes_type(r#"{"bytes":"many"}"#)]
#[case::numeric_created_at(r#"{"created_at":1720000000}"#)]
#[case::not_an_object("[1, 2, 3]")]
#[case::not_json("not json")]
fn a_bad_reply_carries_the_litellm_prefix(#[case] body: &str) {
    let error = decode_create_file(body.as_bytes(), NOW).expect_err("the reply is not a file");
    assert!(matches!(error, ManusFilesError::ParseResponse { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Error parsing Manus file response: ")
    );
}

#[test]
fn retrieve_runs_the_create_reply_rule() {
    let config = plain_config(NOW);
    let file = config
        .transform_retrieve_file_response(
            &response(r#"{"id":"file-9","created_at":"garbage"}"#),
            &mut FileLogging::default(),
            &params("{}"),
        )
        .expect("the reply is a Manus file");
    assert_eq!(file.id, "file-9");
    assert_eq!(file.created_at, NOW);
    assert_eq!(file.purpose, FilesPurpose::Assistants);
}

#[test]
fn retrieve_also_honours_a_stored_first_reply() {
    let config = plain_config(NOW);
    let litellm_params = params(r#"{"initial_file_response":{"id":"stored"}}"#);
    let file = config
        .transform_retrieve_file_response(
            &response(r#"{"id":"raw"}"#),
            &mut FileLogging::default(),
            &litellm_params,
        )
        .expect("the stored reply is a Manus file");
    assert_eq!(file.id, "stored");
}

#[test]
fn a_bad_reply_through_the_trait_keeps_the_status_and_message() {
    let error = plain_config(NOW)
        .transform_create_file_response(
            None,
            &response("not json"),
            &mut FileLogging::default(),
            &FileParams::new(),
        )
        .expect_err("the reply is not a file");
    let FileContractError::Upstream(inner) = error else {
        panic!("the module error becomes the upstream error type");
    };
    assert_eq!(inner.status_code, 500);
    assert!(
        inner
            .message
            .starts_with("Error parsing Manus file response: ")
    );
}
