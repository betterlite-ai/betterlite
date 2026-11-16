#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::first_difference;

#[rstest]
#[case::python_and_rust_exponents("6.25e-05", "6.25e-5", None)]
#[case::exponent_and_plain_decimal("6.25e-05", "0.0000625", None)]
#[case::trailing_zero("2.50", "2.5", None)]
#[case::int_against_float("1", "1.0", Some("/value"))]
#[case::different_floats("0.1", "0.2", Some("/value"))]
#[case::negative_zero("0.0", "-0.0", Some("/value"))]
fn floats_compare_by_value_and_ints_by_text(
    #[case] left: &str,
    #[case] right: &str,
    #[case] expected: Option<&str>,
) {
    let left_value: Value = serde_json::from_str(&format!("{{\"value\": {left}}}")).unwrap();
    let right_value: Value = serde_json::from_str(&format!("{{\"value\": {right}}}")).unwrap();
    assert_eq!(
        first_difference(&left_value, &right_value, "").as_deref(),
        expected
    );
}
