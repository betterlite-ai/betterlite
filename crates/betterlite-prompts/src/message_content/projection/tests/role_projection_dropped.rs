use super::*;
use rstest::rstest;

#[rstest]
#[case::system("system")]
#[case::developer("developer")]
fn message_content_behavior_tests_behavior(#[case] role: &str) {
    assert!(to_content_only_messages(&[message(role, "x")]).is_empty());
}
