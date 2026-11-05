#![cfg(test)]

use reqwest::StatusCode;

use super::shared::{
    DONE_ALLOW, UPLOADED, body_text, data_url, done, sanitizer, vendor, vendor_with,
};
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::messages::process_standalone_images;
use std::time::Duration;

#[tokio::test]
async fn standalone_images_upload_decoded_bytes_named_after_the_mime_type() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let images = [data_url("image/jpeg", "pixels")];
    process_standalone_images(&sanitizer(&server.base), &images, Some("alias"))
        .await
        .expect("allowed");
    let uploads = server.requests("POST");
    let body = body_text(uploads.first().expect("upload"));
    assert!(body.contains(r#"filename="image.jpeg""#));
    assert!(body.contains("pixels"));
}

#[tokio::test]
async fn standalone_images_skip_urls_that_are_not_data_urls() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let images = ["https://example.test/a.png".to_owned()];
    process_standalone_images(&sanitizer(&server.base), &images, None)
        .await
        .expect("skipped");
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn standalone_images_raise_a_400_when_blocked() {
    let server = vendor(UPLOADED, &[done("block", "x")]).await;
    let images = [data_url("image/png", "pixels")];
    let error = process_standalone_images(&sanitizer(&server.base), &images, None)
        .await
        .expect_err("blocked");
    assert!(matches!(
        &error,
        FileSanitizationError::Blocked {
            resource: "Image",
            ..
        }
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn standalone_images_log_and_skip_a_bad_payload() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let images = [
        "data:image/png;base64,!!!".to_owned(),
        "data:image/png".to_owned(),
    ];
    process_standalone_images(&sanitizer(&server.base), &images, None)
        .await
        .expect("swallowed");
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn standalone_images_log_and_skip_a_vendor_failure() {
    let server = vendor_with(StatusCode::INTERNAL_SERVER_ERROR, "{}", &[], Duration::ZERO).await;
    let images = [data_url("image/png", "pixels")];
    process_standalone_images(&sanitizer(&server.base), &images, None)
        .await
        .expect("swallowed");
    assert_eq!(server.requests("POST").len(), 1);
}

#[tokio::test]
async fn standalone_images_raise_vendor_http_exceptions() {
    let server = vendor("{}", &[]).await;
    let images = [data_url("image/png", "pixels")];
    let error = process_standalone_images(&sanitizer(&server.base), &images, None)
        .await
        .expect_err("missing job id");
    assert!(matches!(error, FileSanitizationError::MissingJobId));
}
