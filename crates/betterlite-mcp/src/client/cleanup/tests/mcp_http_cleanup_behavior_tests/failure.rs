use std::sync::Arc;

use rstest::rstest;

use super::super::super::failure::{
    ErrorContext, Failure, McpError, ReadTimeout, as_mcp_read_timeout, first_non_cancelled_cause,
};

struct Label {
    text: String,
}

fn leaf(text: &str) -> (Arc<Label>, Failure<Label>) {
    let error = Arc::new(Label {
        text: text.to_owned(),
    });
    (Arc::clone(&error), Failure::Error(error))
}

#[test]
fn cause_search_is_breadth_first_before_depth() {
    let (deep, deep_failure) = leaf("deep");
    let (shallow, shallow_failure) = leaf("shallow");
    let tree = Failure::Group(vec![Failure::Group(vec![deep_failure]), shallow_failure]);
    let found = first_non_cancelled_cause(&tree, &[]).unwrap();
    assert!(Arc::ptr_eq(&found, &shallow));
    assert!(!Arc::ptr_eq(&found, &deep));
}

#[test]
fn cause_search_keeps_sibling_order() {
    let (first, first_failure) = leaf("first");
    let (_, second_failure) = leaf("second");
    let tree = Failure::Group(vec![first_failure, second_failure]);
    let found = first_non_cancelled_cause(&tree, &[]).unwrap();
    assert!(Arc::ptr_eq(&found, &first));
}

#[test]
fn cause_search_skips_cancelled_leaves() {
    let (real, real_failure) = leaf("real");
    let tree = Failure::Group(vec![Failure::Cancelled, real_failure]);
    let found = first_non_cancelled_cause(&tree, &[]).unwrap();
    assert!(Arc::ptr_eq(&found, &real));
}

#[test]
fn cause_search_skips_recorded_errors_by_identity() {
    let (recorded, recorded_failure) = leaf("same text");
    let (other, other_failure) = leaf("same text");
    let tree = Failure::Group(vec![recorded_failure, other_failure]);
    let found = first_non_cancelled_cause(&tree, &[recorded]).unwrap();
    assert!(Arc::ptr_eq(&found, &other));
    assert_eq!(found.text, "same text");
}

#[test]
fn cause_search_returns_none_when_only_cancelled_and_recorded_remain() {
    let (recorded, recorded_failure) = leaf("recorded");
    let tree = Failure::Group(vec![
        Failure::Cancelled,
        Failure::Group(vec![recorded_failure, Failure::Cancelled]),
    ]);
    assert!(first_non_cancelled_cause(&tree, &[recorded]).is_none());
}

#[test]
fn cause_search_returns_a_lone_error() {
    let (only, only_failure) = leaf("only");
    let found = first_non_cancelled_cause(&only_failure, &[]).unwrap();
    assert!(Arc::ptr_eq(&found, &only));
}

const SDK_CODE: i32 = -32001;
const OTHER_CODE: i32 = -32000;

#[rstest]
#[case::sdk_timeout(
    SDK_CODE,
    Some(ErrorContext::Timeout),
    Some("Request 'tools/list' timed out")
)]
#[case::relayed_error_with_same_code(SDK_CODE, None, None)]
#[case::other_code(OTHER_CODE, Some(ErrorContext::Timeout), None)]
#[case::other_context(SDK_CODE, Some(ErrorContext::Other), None)]
fn read_timeout_needs_the_code_and_a_timeout_context(
    #[case] code: i32,
    #[case] context: Option<ErrorContext>,
    #[case] message: Option<&str>,
) {
    let error = McpError {
        code,
        message: "Request 'tools/list' timed out".to_owned(),
        context,
    };
    let expected = message.map(|text| ReadTimeout {
        message: text.to_owned(),
    });
    assert_eq!(as_mcp_read_timeout(&error), expected);
}
