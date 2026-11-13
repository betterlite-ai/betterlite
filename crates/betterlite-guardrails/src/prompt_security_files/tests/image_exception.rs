#![cfg(test)]

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{
    UPLOADED, data_url, done, image_item, object, sanitizer, text, vendor, vendor_with,
};
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::image::process_image_url_item;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use std::time::Duration;

#[rstest]
#[case::invalid_base64("data:image/png;base64,!!!")]
#[case::truncated_padding("data:image/png;base64,QUJDR")]
#[case::missing_comma("data:image/png;base64")]
#[tokio::test]
async fn image_exception_wraps_a_bad_data_url_as_a_500(#[case] url: &str) {
    let server = vendor(UPLOADED, &[]).await;
    let error = process_image_url_item(&sanitizer(&server.base), image_item(url), None)
        .await
        .expect_err("bad data url");
    assert!(matches!(error, FileSanitizationError::ImageFailed { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.to_string().starts_with("File sanitization failed: "));
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn image_exception_wraps_a_vendor_http_error_as_a_500() {
    let server = vendor_with(StatusCode::BAD_GATEWAY, "{}", &[], Duration::ZERO).await;
    let error = process_image_url_item(
        &sanitizer(&server.base),
        image_item(&data_url("image/png", "pixels")),
        None,
    )
    .await
    .expect_err("vendor failed");
    assert!(matches!(
        &error,
        FileSanitizationError::ImageFailed { source } if matches!(**source, FileSanitizationError::HttpStatus { .. })
    ));
}

#[tokio::test]
async fn image_exception_lets_http_exceptions_through_unwrapped() {
    let missing = vendor("{}", &[]).await;
    let error = process_image_url_item(
        &sanitizer(&missing.base),
        image_item(&data_url("image/png", "pixels")),
        None,
    )
    .await
    .expect_err("missing job id");
    assert!(matches!(error, FileSanitizationError::MissingJobId));
    let blocked = vendor(UPLOADED, &[done("block", "x")]).await;
    let blocked_error = process_image_url_item(
        &sanitizer(&blocked.base),
        image_item(&data_url("image/png", "pixels")),
        None,
    )
    .await
    .expect_err("blocked");
    assert!(matches!(
        blocked_error,
        FileSanitizationError::Blocked { .. }
    ));
}

#[rstest]
#[case::number_image_url(GuardrailValue::UnsignedInteger(7), "image_url")]
#[case::number_url(GuardrailValue::Object(object(&[("url", GuardrailValue::UnsignedInteger(7))])), "url")]
#[tokio::test]
async fn image_exception_rejects_a_value_of_the_wrong_type_with_a_400(
    #[case] image_url: GuardrailValue,
    #[case] field: &str,
) {
    let item = object(&[("type", text("image_url")), ("image_url", image_url)]);
    let error = process_image_url_item(&sanitizer("http://unused.test"), item, None)
        .await
        .expect_err("wrong type");
    assert!(
        matches!(&error, FileSanitizationError::InvalidField { field: named } if *named == field)
    );
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}
