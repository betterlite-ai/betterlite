#![cfg(test)]

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{UPLOADED, sanitizer, vendor};
use crate::prompt_security_files::error::FileSanitizationError;

#[rstest]
#[case::failed(r#"{"status":"failed"}"#, "failed")]
#[case::capitalized(r#"{"status":"Created"}"#, "Created")]
#[case::empty(r#"{"status":""}"#, "")]
#[case::absent(r#"{"content":"x"}"#, "None")]
#[tokio::test]
async fn unexpected_status_is_a_500_that_names_the_status(#[case] poll: &str, #[case] shown: &str) {
    let server = vendor(UPLOADED, &[poll.to_owned()]).await;
    let error = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect_err("unexpected");
    assert!(
        matches!(&error, FileSanitizationError::UnexpectedStatus { status } if status == shown)
    );
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.to_string(),
        format!("Unexpected sanitization status: {shown}")
    );
    assert_eq!(server.requests("GET").len(), 1);
}
