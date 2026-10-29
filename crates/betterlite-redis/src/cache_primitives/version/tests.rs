#![cfg(test)]

use rstest::rstest;

use super::{RedisVersionValue, parse_major_version};

#[rstest]
#[case::dotted_string(RedisVersionValue::Text("7.0.0".to_owned()), 7)]
#[case::bare_digit_string(RedisVersionValue::Text("6".to_owned()), 6)]
#[case::rc_suffix(RedisVersionValue::Text("7.0.0-rc1".to_owned()), 7)]
#[case::surrounding_whitespace(RedisVersionValue::Text(" 7.0.0 ".to_owned()), 7)]
#[case::valkey_float(RedisVersionValue::Float(7.0), 7)]
#[case::valkey_float_six(RedisVersionValue::Float(6.0), 6)]
#[case::plain_integer(RedisVersionValue::Integer(7), 7)]
fn parses_known_formats(#[case] value: RedisVersionValue, #[case] expected: u32) {
    assert_eq!(parse_major_version(&value, 9), expected);
}

#[rstest]
#[case::unknown_marker(RedisVersionValue::Text("Unknown".to_owned()))]
#[case::empty_string(RedisVersionValue::Text(String::new()))]
#[case::non_numeric_word(RedisVersionValue::Text("latest".to_owned()))]
#[case::v_prefixed_string(RedisVersionValue::Text("v6".to_owned()))]
fn falls_back_to_default_on_malformed_input(#[case] value: RedisVersionValue) {
    assert_eq!(parse_major_version(&value, 9), 9);
}
