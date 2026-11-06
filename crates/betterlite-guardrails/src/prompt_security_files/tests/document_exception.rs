#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{UPLOADED, data_url, done, object, sanitizer, text, vendor, vendor_with};
use crate::prompt_security_files::document::process_document_item;
use crate::prompt_security_files::error::FileSanitizationError;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

fn document(fields: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    object(&[
        ("type", text("document")),
        ("document", GuardrailValue::Object(object(fields))),
    ])
}

#[rstest]
#[case::invalid_base64_data(document(&[("data", text("!!!"))]))]
#[case::invalid_base64_url(document(&[("url", text("data:application/pdf;base64,!!!"))]))]
#[case::missing_comma(document(&[("url", text("data:application/pdf"))]))]
#[tokio::test]
async fn document_exception_wraps_bad_base64_as_a_500(
    #[case] item: BTreeMap<String, GuardrailValue>,
) {
    let server = vendor(UPLOADED, &[]).await;
    let error = process_document_item(&sanitizer(&server.base), item, None)
        .await
        .expect_err("bad payload");
    assert!(matches!(
        error,
        FileSanitizationError::DocumentFailed { .. }
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        error
            .to_string()
            .starts_with("Document sanitization failed: ")
    );
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn document_exception_wraps_a_vendor_http_error_as_a_500() {
    let server = vendor_with(StatusCode::BAD_GATEWAY, "{}", &[], Duration::ZERO).await;
    let item = document(&[("url", text(&data_url("application/pdf", "hello")))]);
    let error = process_document_item(&sanitizer(&server.base), item, None)
        .await
        .expect_err("vendor failed");
    assert!(matches!(
        &error,
        FileSanitizationError::DocumentFailed { source } if matches!(**source, FileSanitizationError::HttpStatus { .. })
    ));
}

#[tokio::test]
async fn document_exception_lets_http_exceptions_through_unwrapped() {
    let missing = vendor("{}", &[]).await;
    let item = document(&[("data", text("aGVsbG8="))]);
    let error = process_document_item(&sanitizer(&missing.base), item.clone(), None)
        .await
        .expect_err("missing job id");
    assert!(matches!(error, FileSanitizationError::MissingJobId));
    let blocked = vendor(UPLOADED, &[done("block", "x")]).await;
    let blocked_error = process_document_item(&sanitizer(&blocked.base), item, None)
        .await
        .expect_err("blocked");
    assert!(matches!(
        blocked_error,
        FileSanitizationError::Blocked { .. }
    ));
}

#[rstest]
#[case::url(document(&[("url", GuardrailValue::UnsignedInteger(1))]), "url")]
#[case::data(document(&[("data", GuardrailValue::Boolean(true))]), "data")]
#[case::mime_type(document(&[("data", text("aGVsbG8=")), ("mime_type", GuardrailValue::UnsignedInteger(1))]), "mime_type")]
#[tokio::test]
async fn document_exception_rejects_a_value_of_the_wrong_type_with_a_400(
    #[case] item: BTreeMap<String, GuardrailValue>,
    #[case] field: &str,
) {
    let error = process_document_item(&sanitizer("http://unused.test"), item, None)
        .await
        .expect_err("wrong type");
    assert!(
        matches!(&error, FileSanitizationError::InvalidField { field: named } if *named == field)
    );
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
