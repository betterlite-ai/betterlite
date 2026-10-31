#![cfg(test)]

use super::TokenCount;

fn from_json(source: &str) -> TokenCount {
    serde_json::from_str(source).expect("valid JSON")
}

#[test]
fn integer_keeps_its_value() {
    assert_eq!(from_json("42"), TokenCount::Value(42));
    assert_eq!(from_json("-7"), TokenCount::Value(-7));
}

#[test]
fn bool_counts_as_zero_or_one() {
    assert_eq!(from_json("true"), TokenCount::Value(1));
    assert_eq!(from_json("false"), TokenCount::Value(0));
}

#[test]
fn non_integer_shapes_count_as_zero() {
    assert_eq!(from_json("1.5"), TokenCount::Value(0));
    assert_eq!(from_json("\"3\""), TokenCount::Value(0));
    assert_eq!(from_json("null"), TokenCount::Value(0));
    assert_eq!(from_json("{}"), TokenCount::Value(0));
    assert_eq!(from_json("[1, 2]"), TokenCount::Value(0));
}

#[test]
fn integer_beyond_i64_is_retained_for_transform_error() {
    assert!(matches!(
        from_json("9223372036854775808"),
        TokenCount::Overflow(_)
    ));
    assert!(matches!(
        from_json("-9223372036854775809"),
        TokenCount::Overflow(_)
    ));
}
