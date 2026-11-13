#![cfg(test)]

use reqwest::StatusCode;

use super::shared::{CREATED, UPLOADED, sanitizer, vendor};
use crate::prompt_security_files::error::FileSanitizationError;

#[tokio::test]
async fn poll_exhausted_is_a_408_even_when_fail_open_is_true() {
    let server = vendor(UPLOADED, &[CREATED.to_owned()]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.max_poll_attempts = 3;
    assert!(guardrail.settings.fail_open);
    let error = guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect_err("exhausted");
    assert!(matches!(error, FileSanitizationError::Timeout));
    assert_eq!(error.status_code(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(error.to_string(), "File sanitization timeout");
    assert_eq!(server.requests("GET").len(), 3);
}
