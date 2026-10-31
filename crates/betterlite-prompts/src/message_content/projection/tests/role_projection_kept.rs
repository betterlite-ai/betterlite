use super::*;
use rstest::rstest;

#[rstest]
#[case::user("user")]
#[case::tool("tool")]
#[case::function("function")]
#[case::assistant("assistant")]
fn message_content_behavior_tests_behavior(#[case] role: &str) {
    assert_eq!(to_content_only_messages(&[message(role, "x")]).len(), 1);
}
