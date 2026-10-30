use super::*;

#[rstest]
#[case::positive("content_policy_violation", true)]
#[case::negative("ordinary provider failure", false)]
fn error_facts_behavior_tests_behavior(#[case] message: &str, #[case] expected: bool) {
    assert_eq!(ExceptionCheckers::azure_policy(message), expected);
}
