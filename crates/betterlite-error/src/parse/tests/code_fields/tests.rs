#![cfg(test)]

use super::*;

#[rstest]
#[case::unrelated_message("{\"message\":42,\"error\":{\"code\":429}}", Some(429))]
#[case::spaced_code("{\"error\":{\"code\":\" +429 \"}}", Some(429))]
fn error_facts_behavior_tests_behavior(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(get_body_error_code(body), expected);
}
