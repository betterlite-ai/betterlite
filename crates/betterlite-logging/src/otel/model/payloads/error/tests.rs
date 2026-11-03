#![cfg(test)]

use super::*;
#[test]
fn error_uses_error_string_fallback() {
    let payload = serde_json::from_value::<BTreeMap<String, Value>>(
        serde_json::json!({"error_str":"failed"}),
    )
    .unwrap();
    assert_eq!(
        SpanError::from_payload(&payload).and_then(|error| error.message),
        Some("failed".to_owned())
    );
}
