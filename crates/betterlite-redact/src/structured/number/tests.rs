#![cfg(test)]

use rstest::rstest;
use serde_json::Number;

#[rstest]
#[case::integer("12345", "12345")]
#[case::negative_zero("-0.0", "-0.0")]
#[case::plain("0.0001", "0.0001")]
#[case::small("1e-5", "1e-05")]
#[case::small_six("1e-6", "1e-06")]
#[case::negative("-1.2345e-5", "-1.2345e-05")]
#[case::tiny("1.2e-7", "1.2e-07")]
#[case::long_exponent("1e-100", "1e-100")]
#[case::large("1e20", "1e+20")]
fn structured_redaction_behavior_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let number = serde_json::from_str::<Number>(source).unwrap();
    assert_eq!(super::text(&number), expected);
}
