#![cfg(test)]

use crate::envelope::error_body_call_id;
use rstest::rstest;

#[rstest]
#[case::enabled(true, Some("request-id"), Some("request-id"))]
#[case::preserves_id(true, Some("different-id"), Some("different-id"))]
#[case::disabled(false, Some("request-id"), None)]
#[case::absent(true, None, None)]
#[case::empty(true, Some(""), None)]
fn error_normalization_tests_behavior(
    #[case] enabled: bool,
    #[case] id: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(error_body_call_id(enabled, id), expected);
}
