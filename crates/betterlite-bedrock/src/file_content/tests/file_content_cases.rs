use super::super::{FileContentError, FileContentRequest, RawResponse, transform_file_response};
use crate::bedrock_settings::error_response_text;
use rstest::rstest;
use std::error::Error as _;

fn default_headers() -> Vec<(String, String)> {
    vec![
        ("content-type".to_owned(), "application/x-custom".to_owned()),
        (
            "content-disposition".to_owned(),
            "attachment; filename=x".to_owned(),
        ),
    ]
}

fn response_with_headers(
    status_code: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
) -> RawResponse {
    RawResponse {
        status_code,
        body,
        headers,
        request: FileContentRequest {
            method: "GET".to_owned(),
            url: "https://bedrock.example/files/id/content".to_owned(),
        },
    }
}

#[rstest]
#[case::invalid_utf8(vec![0, 255, 10])]
#[case::nul_bytes(vec![b'{', 0, b'}'])]
#[case::json_looking(br#"{"records":[{"ok":true}]}"#.to_vec())]
fn raw_binary_reply(#[case] body: Vec<u8>) {
    let expected = response_with_headers(200, body, default_headers());
    let actual = transform_file_response(expected.clone()).expect("success");
    assert_eq!(actual.response, expected);
    assert!(actual.hidden_params.is_empty());
    assert_eq!(
        actual.logging_summary().num_bytes,
        actual.response.body.len()
    );
}

#[rstest]
fn empty_binary_reply() {
    let expected = response_with_headers(204, Vec::new(), default_headers());
    let actual = transform_file_response(expected.clone()).expect("success");
    assert_eq!(actual.response, expected);
    assert!(actual.logging_summary().content_type.is_some());
}

#[rstest]
#[case::text(b"bad request".to_vec(), "bad request")]
#[case::empty(Vec::new(), "")]
#[case::invalid_utf8(vec![0, 255, 10], "\u{0}\u{fffd}\n")]
fn failed_binary_reply(#[case] body: Vec<u8>, #[case] message: &str) {
    let expected = response_with_headers(500, body.clone(), default_headers());
    let outcome = transform_file_response(expected.clone()).expect_err("failure");
    assert!(outcome.source().is_some());
    let FileContentError::Bedrock {
        status,
        body: raw_body,
        error,
    } = outcome
    else {
        panic!("expected BedrockError");
    };
    assert_eq!(status, 500);
    assert_eq!(raw_body, body);
    assert_eq!(error.cause.error.status_code, 500);
    assert_eq!(error.cause.error.message, message);
    assert_eq!(error.cause.response_headers, expected.headers);
    assert_eq!(error_response_text(&error.cause.response_body), message);
}

#[rstest]
#[case::declared_charset(vec![0xE9], "iso-8859-1", "\u{e9}")]
#[case::iso_8859_1_byte_0x80(vec![0x80], "iso-8859-1", "\u{80}")]
#[case::ascii_invalid_byte(vec![b'a', 0x80], "us-ascii", "a\u{fffd}")]
#[case::ascii_valid_utf8_bytes(vec![0xC3, 0xA9], "us-ascii", "\u{fffd}\u{fffd}")]
#[case::windows_1252(vec![0x80], "windows-1252", "\u{20ac}")]
#[case::utf_16le(vec![b'h', 0, b'i', 0], "UTF-16LE", "hi")]
#[case::windows_1252_alias(vec![0x80], "cp1252", "\u{20ac}")]
#[case::utf_16le_alias(vec![b'h', 0, b'i', 0], "utf_16_le", "hi")]
fn failed_binary_reply_decodes_declared_charset(
    #[case] body: Vec<u8>,
    #[case] charset: &str,
    #[case] message: &str,
) {
    let expected = response_with_headers(
        500,
        body,
        vec![(
            "Content-Type".to_owned(),
            format!("text/plain; charset={charset}"),
        )],
    );
    let outcome = transform_file_response(expected).expect_err("failure");
    let FileContentError::Bedrock { error, .. } = outcome else {
        panic!("expected BedrockError");
    };
    assert_eq!(error.cause.error.message, message);
}

#[rstest]
fn raw_binary_reply_preserves_repeated_headers() {
    let expected = response_with_headers(
        200,
        vec![1, 2, 3],
        vec![
            ("Content-Type".to_owned(), "application/x-custom".to_owned()),
            ("content-type".to_owned(), "text/plain".to_owned()),
        ],
    );
    let actual = transform_file_response(expected.clone()).expect("success");
    assert_eq!(actual.response.headers, expected.headers);
    assert_eq!(
        actual.logging_summary().content_type,
        Some("application/x-custom, text/plain".to_owned())
    );
}

#[rstest]
#[case::informational(101)]
#[case::redirect(302)]
#[case::no_content(204)]
fn non_error_statuses(#[case] status_code: u16) {
    let expected = response_with_headers(status_code, Vec::new(), default_headers());
    let actual = transform_file_response(expected.clone()).expect("non-error response");
    assert_eq!(actual.response, expected);
}

#[test]
fn case_1_raw_binary_reply() {
    raw_binary_reply(vec![0, 255, 10]);
}

#[test]
fn case_1_empty_binary_reply() {
    empty_binary_reply();
}

#[test]
fn case_1_failed_binary_reply() {
    failed_binary_reply(b"bad request".to_vec(), "bad request");
}

#[test]
fn case_1_non_error_statuses() {
    non_error_statuses(302);
}
