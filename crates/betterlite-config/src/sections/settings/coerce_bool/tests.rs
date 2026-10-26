#![cfg(test)]

use super::super::JsonValue;

#[test]
fn normalizes_strings_and_json_values() {
    assert_eq!(super::coerce_bool(JsonValue::Text(String::from("TRUE"))), JsonValue::Boolean(true));
    assert_eq!(super::coerce_bool(JsonValue::Text(String::from("false"))), JsonValue::Boolean(false));
    assert_eq!(super::coerce_bool(JsonValue::SignedInteger(3)), JsonValue::Boolean(true));
    assert_eq!(super::coerce_bool(JsonValue::Null), JsonValue::Null);
}
