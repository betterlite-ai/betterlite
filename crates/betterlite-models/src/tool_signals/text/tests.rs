#![cfg(test)]

use rstest::rstest;

use super::{digit_magnitude, is_float_literal, valid_underscored_digits};

#[rstest]
#[case::plain("123", true)]
#[case::underscored("1_000", true)]
#[case::leading_underscore("_1", false)]
#[case::trailing_underscore("1_", false)]
#[case::doubled_underscore("1__0", false)]
#[case::empty("", false)]
#[case::signed("-1", false)]
fn digit_runs_accept_single_inner_underscores(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(valid_underscored_digits(text), expected);
}

#[rstest]
#[case::plain("123", Some(123))]
#[case::underscored("1_000", Some(1000))]
#[case::too_large("999999999999999999999999999999999999999999", None)]
fn magnitudes_read_digits_without_underscores(#[case] text: &str, #[case] expected: Option<i128>) {
    assert_eq!(digit_magnitude(text), expected);
}

#[rstest]
#[case::integer("3", true)]
#[case::decimal("3.5", true)]
#[case::leading_dot(".5", true)]
#[case::trailing_dot("5.", true)]
#[case::exponent("1e3", true)]
#[case::signed_exponent("1e-3", true)]
#[case::underscored("1_0.0_1", true)]
#[case::infinity("Infinity", true)]
#[case::not_a_number("NaN", true)]
#[case::hex("0x10", false)]
#[case::empty("", false)]
#[case::bare_dot(".", false)]
#[case::invalid_integer_part("a.5", false)]
#[case::invalid_fraction_part("5.a", false)]
#[case::empty_exponent("1e", false)]
#[case::signed_sign("+-1", false)]
#[case::double_underscore("1__0", false)]
fn float_literals_follow_the_string_grammar(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_float_literal(text), expected);
}
