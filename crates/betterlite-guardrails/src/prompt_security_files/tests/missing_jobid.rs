#![cfg(test)]

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{sanitizer, vendor};
use crate::prompt_security_files::error::FileSanitizationError;

#[rstest]
#[case::no_key("{}")]
#[case::null_job_id(r#"{"jobId":null}"#)]
#[case::empty_job_id(r#"{"jobId":""}"#)]
#[tokio::test]
async fn missing_jobid_is_a_500_and_never_polls(#[case] upload: &str) {
    let server = vendor(upload, &[]).await;
    let error = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect_err("no job id");
    assert!(matches!(error, FileSanitizationError::MissingJobId));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.to_string(),
        "Failed to get jobId from Prompt Security"
    );
    assert!(server.requests("GET").is_empty());
}
