use rstest::rstest;

use crate::sso_polling::polling::{polling_error_message, response_error_detail};

use super::fixtures::response;

#[test]
fn error_extracts_a_non_empty_detail_string() {
    let detail = response_error_detail(&response(500, r#"{"detail":"boom"}"#));

    assert_eq!(detail.as_deref(), Some("boom"));
}

#[test]
fn error_ignores_an_empty_detail() {
    let detail = response_error_detail(&response(400, r#"{"detail":""}"#));

    assert_eq!(detail, None);
}

#[test]
fn error_ignores_a_non_string_detail() {
    let detail = response_error_detail(&response(400, r#"{"detail":5}"#));

    assert_eq!(detail, None);
}

#[test]
fn error_ignores_a_body_without_a_detail() {
    let detail = response_error_detail(&response(500, r#"{"error":"server_error"}"#));

    assert_eq!(detail, None);
}

#[rstest]
#[case::array(r#"["detail"]"#)]
#[case::string(r#""detail""#)]
fn error_ignores_a_non_object_body(#[case] body: &str) {
    let detail = response_error_detail(&response(500, body));

    assert_eq!(detail, None);
}

#[test]
fn error_ignores_a_body_that_is_not_json() {
    let detail = response_error_detail(&response(502, "Bad Gateway"));

    assert_eq!(detail, None);
}

#[test]
fn error_ignores_trailing_bytes_after_the_json_object() {
    let detail = response_error_detail(&response(500, r#"{"detail":"boom"} trailing"#));

    assert_eq!(detail, None);
}

#[test]
fn error_message_includes_the_detail_when_present() {
    let message = polling_error_message(&response(500, r#"{"detail":"boom"}"#));

    assert_eq!(message, "Polling error: HTTP 500: boom");
}

#[test]
fn error_message_stops_at_the_status_without_a_detail() {
    let message = polling_error_message(&response(502, "Bad Gateway"));

    assert_eq!(message, "Polling error: HTTP 502");
}
