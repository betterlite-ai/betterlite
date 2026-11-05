#![cfg(test)]

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{
    DONE_ALLOW, UPLOADED, body_text, data_url, done, image_item, object, sanitizer, text, vendor,
};
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::image::process_image_url_item;

const REDACTED_BASE64: &str = "UkVEQUNURUQ=";

#[rstest]
#[case::png("image/png", "image.png")]
#[case::jpeg("image/jpeg", "image.jpeg")]
#[case::svg("image/svg+xml", "image.svg+xml")]
#[tokio::test]
async fn image_url_uploads_decoded_bytes_named_after_the_mime_type(
    #[case] mime: &str,
    #[case] filename: &str,
) {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let item = image_item(&data_url(mime, "pixels"));
    let processed = process_image_url_item(&sanitizer(&server.base), item.clone(), Some("alias"))
        .await
        .expect("allowed");
    assert_eq!(processed, item);
    let uploads = server.requests("POST");
    let body = body_text(uploads.first().expect("upload"));
    assert!(body.contains(&format!(r#"filename="{filename}""#)));
    assert!(body.contains("pixels"));
}

#[tokio::test]
async fn image_url_leaves_remote_urls_alone_without_calling_the_vendor() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let item = image_item("https://example.test/a.png");
    let processed = process_image_url_item(&sanitizer(&server.base), item.clone(), None)
        .await
        .expect("untouched");
    assert_eq!(processed, item);
    assert!(server.requests("POST").is_empty());
}

#[tokio::test]
async fn image_url_replaces_the_url_when_modify_is_allowed() {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let processed = process_image_url_item(
        &guardrail,
        image_item(&data_url("image/png", "pixels")),
        None,
    )
    .await
    .expect("modified");
    let expected = format!("data:image/png;base64,{REDACTED_BASE64}");
    assert_eq!(processed, image_item(&expected));
}

#[tokio::test]
async fn image_url_replaces_a_string_valued_image_url() {
    let server = vendor(UPLOADED, &[done("modify", "REDACTED")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = object(&[
        ("type", text("image_url")),
        ("image_url", text(&data_url("image/png", "pixels"))),
    ]);
    let processed = process_image_url_item(&guardrail, item, None)
        .await
        .expect("modified");
    let expected = text(&format!("data:image/png;base64,{REDACTED_BASE64}"));
    assert_eq!(processed["image_url"], expected);
}

#[tokio::test]
async fn image_url_keeps_the_url_when_modify_returns_no_content() {
    let server = vendor(
        UPLOADED,
        &[r#"{"status":"done","metadata":{"action":"modify"}}"#.to_owned()],
    )
    .await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.block_on_file_modify = false;
    let item = image_item(&data_url("image/png", "pixels"));
    let processed = process_image_url_item(&guardrail, item.clone(), None)
        .await
        .expect("kept");
    assert_eq!(processed, item);
}

#[rstest]
#[case::block("block")]
#[case::modify_with_default_flag("modify")]
#[tokio::test]
async fn image_url_raises_a_400_naming_the_violations(#[case] action: &str) {
    let server = vendor(UPLOADED, &[done(action, "REDACTED")]).await;
    let error = process_image_url_item(
        &sanitizer(&server.base),
        image_item(&data_url("image/png", "pixels")),
        None,
    )
    .await
    .expect_err("blocked");
    assert!(matches!(
        &error,
        FileSanitizationError::Blocked {
            resource: "File",
            ..
        }
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.to_string(),
        "File blocked by Prompt Security. Violations: secret, pii"
    );
}

#[tokio::test]
async fn image_url_drops_whitespace_inside_the_base64_payload() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let item = image_item("data:image/png;base64,cGl4\n ZWxz\r\n");
    process_image_url_item(&sanitizer(&server.base), item, None)
        .await
        .expect("allowed");
    let uploads = server.requests("POST");
    assert!(body_text(uploads.first().expect("upload")).contains("pixels"));
}
