#![cfg(test)]

use std::time::Duration;

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::{DONE_ALLOW, UPLOADED, sanitizer, vendor, vendor_with};
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::wire::{Action, SanitizeResult};

const SLOW: Duration = Duration::from_millis(400);
const SHORT: Duration = Duration::from_millis(60);

#[rstest]
#[case::overall_timeout(true)]
#[case::request_timeout(false)]
#[tokio::test]
async fn timeout_wrapper_fails_open_with_an_empty_allow(#[case] overall: bool) {
    let server = vendor_with(StatusCode::OK, UPLOADED, &[DONE_ALLOW.to_owned()], SLOW).await;
    let mut guardrail = sanitizer(&server.base);
    assert!(guardrail.settings.fail_open);
    if overall {
        guardrail.settings.file_sanitization_timeout = SHORT;
    } else {
        guardrail.settings.request_timeout = SHORT;
    }
    let result = guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("fails open");
    assert_eq!(
        result,
        SanitizeResult {
            action: Action::Allow,
            content: None,
            violations: Vec::new(),
        }
    );
}

#[rstest]
#[case::overall_timeout(true)]
#[case::request_timeout(false)]
#[tokio::test]
async fn timeout_wrapper_fails_closed_with_a_408(#[case] overall: bool) {
    let server = vendor_with(StatusCode::OK, UPLOADED, &[DONE_ALLOW.to_owned()], SLOW).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.fail_open = false;
    if overall {
        guardrail.settings.file_sanitization_timeout = SHORT;
    } else {
        guardrail.settings.request_timeout = SHORT;
    }
    let error = guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect_err("fails closed");
    assert!(matches!(error, FileSanitizationError::Timeout));
    assert_eq!(error.status_code(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn timeout_wrapper_returns_the_result_when_nothing_times_out() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let result = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert_eq!(result.action, Action::Allow);
}

#[tokio::test]
async fn timeout_wrapper_does_not_swallow_other_failures_when_failing_open() {
    let server = vendor_with(StatusCode::BAD_GATEWAY, "{}", &[], Duration::ZERO).await;
    let guardrail = sanitizer(&server.base);
    assert!(guardrail.settings.fail_open);
    let error = guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect_err("vendor error");
    assert!(
        matches!(error, FileSanitizationError::HttpStatus { status } if status == StatusCode::BAD_GATEWAY)
    );
}
