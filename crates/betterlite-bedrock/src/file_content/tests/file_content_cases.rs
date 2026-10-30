use super::super::{FileContentError, FileContentRequest, RawResponse, transform_file_response};
use rstest::rstest;
use std::collections::BTreeMap;

fn response(status_code: u16, body: Vec<u8>) -> RawResponse {
    RawResponse {
        status_code,
        body,
        headers: BTreeMap::from([
            ("content-type".to_owned(), "application/x-custom".to_owned()),
            (
                "content-disposition".to_owned(),
                "attachment; filename=x".to_owned(),
            ),
        ]),
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
    let expected = response(200, body);
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
    let expected = response(204, Vec::new());
    let actual = transform_file_response(expected.clone()).expect("success");
    assert_eq!(actual.response, expected);
    assert!(actual.logging_summary().content_type.is_some());
}

#[rstest]
#[case::text(b"bad request".to_vec(), "bad request")]
#[case::empty(Vec::new(), "")]
#[case::invalid_utf8(vec![0, 255, 10], "\u{0}\u{fffd}\n")]
fn failed_binary_reply(#[case] body: Vec<u8>, #[case] message: &str) {
    let expected = response(500, body);
    let outcome = transform_file_response(expected.clone()).expect_err("failure");
    let FileContentError::Bedrock(bedrock_error) = outcome;
    assert_eq!(bedrock_error.status_code, 500);
    assert_eq!(bedrock_error.message, message);
    assert_eq!(bedrock_error.headers, expected.headers);
    assert_eq!(bedrock_error.response, expected);
}

#[rstest]
#[case::informational(101)]
#[case::redirect(302)]
#[case::no_content(204)]
fn non_error_statuses(#[case] status_code: u16) {
    let expected = response(status_code, Vec::new());
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
