#![cfg(test)]

use rstest::rstest;

use super::output_cap::parse_output_cap_value;
use super::support::value;

#[rstest]
#[case::integer("4096", Some(4096))]
#[case::truncated_float("4.5", Some(4))]
#[case::negative_float("-4.9", Some(-4))]
#[case::integer_text(r#""4096""#, Some(4096))]
#[case::padded_text(r#"" 4096 ""#, Some(4096))]
#[case::float_text(r#""4.5""#, Some(4))]
#[case::exponent_text(r#""1e3""#, Some(1000))]
#[case::true_bool("true", None)]
#[case::false_bool("false", None)]
#[case::null("null", None)]
#[case::list("[4096]", None)]
#[case::object(r#"{"max_tokens":4096}"#, None)]
#[case::words(r#""abc""#, None)]
#[case::empty_text(r#""""#, None)]
#[case::bool_text(r#""true""#, None)]
#[case::not_a_number(r#""nan""#, None)]
#[case::infinite_text(r#""inf""#, None)]
#[case::huge_number("1e300", Some(i64::MAX))]
#[case::huge_integer("18446744073709551616", Some(i64::MAX))]
#[case::huge_negative("-1e300", Some(i64::MIN))]
fn parsed_caps_match_python(#[case] raw: &str, #[case] expected: Option<i64>) {
    assert_eq!(parse_output_cap_value(&value(raw)), expected);
}
