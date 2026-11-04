#![cfg(test)]

use super::{detect_first_expected_role, get_completion_messages};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::empty(json!([]), Some("user"))]
#[case::system_then_user(json!([{"role":"system"},{"role":"user"}]), Some("assistant"))]
#[case::assistant_first(json!([{"role":"assistant"}]), Some("user"))]
#[case::only_system(json!([{"role":"system"}]), None)]
fn message_normalization_message_tests_behavior(
    #[case] input: Value,
    #[case] expected: Option<&str>,
) {
    let messages = input.as_array().cloned().unwrap_or_default();
    assert_eq!(detect_first_expected_role(&messages), expected);
}

#[rstest]
#[case::continuation(json!([{"role":"assistant","content":"a"}]), true, 3)]
#[case::disabled(json!([{"role":"assistant"}]), false, 1)]
#[case::empty(json!([]), true, 0)]
fn message_normalization_message_tests_completion(
    #[case] input: Value,
    #[case] ensure: bool,
    #[case] length: usize,
) {
    let messages = input.as_array().cloned().unwrap_or_default();
    let original = messages.clone();
    let result = get_completion_messages(&messages, None, None, ensure);
    assert_eq!(result.len(), length);
    assert_eq!(messages, original);
}
