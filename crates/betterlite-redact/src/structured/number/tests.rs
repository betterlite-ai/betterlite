#![cfg(test)]

use crate::structured::parse::parse_payload;
use rstest::rstest;

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
#[case::trailing_zero("1.50", "1.5")]
#[case::exponent_coefficient("1.0e20", "1e+20")]
#[case::float_integer("1.0", "1.0")]
#[case::exponent_integer("1e0", "1.0")]
#[case::positive_threshold("1e16", "1e+16")]
#[case::below_threshold("1e15", "1000000000000000.0")]
#[case::positive_overflow("1e400", "inf")]
#[case::negative_overflow("-1e400", "-inf")]
#[case::integer_zero("-0", "0")]
#[case::unsigned("18446744073709551615", "18446744073709551615")]
#[case::big_integer("123456789012345678901234567890", "123456789012345678901234567890")]
#[case::big_negative("-123456789012345678901234567890", "-123456789012345678901234567890")]
fn structured_redaction_behavior_tests_behavior(#[case] source: &str, #[case] expected: &str) {
    let payload = parse_payload(source.as_bytes()).unwrap();
    assert_eq!(super::text(payload.as_number().unwrap()), expected);
}
