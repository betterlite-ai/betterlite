#![cfg(test)]

use super::*;

#[rstest]
#[case::content_policy("content_policy_violation", true)]
#[case::responsible_ai("ResponsibleAIPolicyViolation", true)]
#[case::ordinary("provider rejected the request", false)]
fn error_facts_behavior_tests_behavior(#[case] message: &str, #[case] expected: bool) {
    assert_eq!(ExceptionCheckers::azure_policy(message), expected);
}
