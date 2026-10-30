#![cfg(test)]

use super::TokenCount;

fn from_json(source: &str) -> TokenCount {
    serde_json::from_str(source).expect("valid JSON")
}

#[test]
fn integer_keeps_its_value() {
    assert_eq!(from_json("42").0, 42);
    assert_eq!(from_json("-7").0, -7);
}

#[test]
fn bool_counts_as_zero_or_one() {
    assert_eq!(from_json("true").0, 1);
    assert_eq!(from_json("false").0, 0);
}

#[test]
fn non_integer_shapes_count_as_zero() {
    assert_eq!(from_json("1.5").0, 0);
    assert_eq!(from_json("\"3\"").0, 0);
    assert_eq!(from_json("null").0, 0);
    assert_eq!(from_json("{}").0, 0);
    assert_eq!(from_json("[1, 2]").0, 0);
}

#[test]
fn integer_beyond_i64_is_an_overflow_error() {
    let error = serde_json::from_str::<TokenCount>("9223372036854775808").unwrap_err();
    assert!(error.to_string().contains("signed 64-bit"));
    serde_json::from_str::<TokenCount>("-9223372036854775809").unwrap_err();
}
