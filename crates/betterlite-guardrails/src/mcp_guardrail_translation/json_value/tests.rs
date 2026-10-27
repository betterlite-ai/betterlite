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
        serde_json::from_str(r#"{"$serde_json::private::Number":"123"}"#).unwrap();
    assert!(
        matches!(value, McpValue::Object(entries) if entries.contains_key("$serde_json::private::Number"))
    );
}
