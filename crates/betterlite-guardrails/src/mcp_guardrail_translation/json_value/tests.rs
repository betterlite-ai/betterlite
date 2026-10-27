#![cfg(test)]

use super::McpValue;

#[test]
fn number_stays_a_number() {
    let value: McpValue = serde_json::from_str("123").unwrap();
    assert!(matches!(value, McpValue::Number(number) if number.to_string() == "123"));
}

#[test]
fn private_number_object_stays_an_object() {
    let value: McpValue =
        serde_json::from_str(r#"{"$serde_json::private::Number":"not a number"}"#).unwrap();
    assert!(
        matches!(value, McpValue::Object(entries) if entries.contains_key("$serde_json::private::Number"))
    );
}

#[test]
fn decimal_number_round_trips_under_workspace_features() {
    let value: McpValue = serde_json::from_str(r#"{"x":2.5}"#).unwrap();
    assert!(
        matches!(&value, McpValue::Object(entries) if matches!(entries.get("x"), Some(McpValue::Number(number)) if number.to_string() == "2.5"))
    );
    assert_eq!(serde_json::to_string(&value).unwrap(), r#"{"x":2.5}"#);
}
