#![cfg(test)]

use std::error::Error as _;

use reqwest::StatusCode;

use super::{GuardrailRaisedException, OvalixBlockedException};
use crate::ovalix::checkpoint::CheckpointError;

#[test]
fn the_blocked_exception_status_is_400() {
    let status = OvalixBlockedException::STATUS_CODE;
    assert_eq!(status, 400);
}

#[test]
fn the_blocked_exception_marks_blocked_content() {
    let blocked_content = OvalixBlockedException::BLOCKED_CONTENT;
    assert!(blocked_content);
    let exception = OvalixBlockedException {
        guardrail_name: "ovalix".to_owned(),
        message: "no".to_owned(),
    };
    assert_eq!(exception.to_string(), "no");
}

#[test]
fn the_raised_exception_status_is_400() {
    let status = GuardrailRaisedException::STATUS_CODE;
    assert_eq!(status, 400);
}

#[test]
fn the_raised_exception_does_not_mark_blocked_content() {
    let blocked_content = GuardrailRaisedException::BLOCKED_CONTENT;
    assert!(!blocked_content);
}

#[test]
fn the_raised_exception_keeps_its_cause() {
    let exception = GuardrailRaisedException {
        guardrail_name: "ovalix".to_owned(),
        source: CheckpointError::Status {
            status: StatusCode::BAD_GATEWAY,
        },
    };
    assert!(
        exception
            .to_string()
            .starts_with("Ovalix guardrail error: the Ovalix Tracker returned status 502")
    );
    assert!(exception.source().is_some());
}
