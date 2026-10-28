#![cfg(test)]

use super::*;

#[test]
fn non_message_payloads_use_attribute_truncation() {
    assert_eq!(truncate_payload("plain"), "plain");
    assert_eq!(truncate_payload("[]"), "[]");
    assert!(
        truncate_payload(&format!("{}x", "a".repeat(OTLP_MAX_ATTRIBUTE_VALUE_BYTES)))
            .contains("truncated 1 bytes")
    );
}

#[test]
fn message_payload_keeps_first_marker_and_newest() {
    let message = "x".repeat(40_000);
    let payload = serde_json::to_string(&vec![
        serde_json::json!({"role":"user", "content":"first"}),
        serde_json::json!({"role":"user", "content":message}),
        serde_json::json!({"role":"assistant", "content":"last"}),
    ])
    .unwrap();
    let output = truncate_payload(&payload);
    assert!(output.starts_with("[{\"role\":\"user\",\"content\":\"first\"}"));
    assert!(output.contains("earlier messages truncated"));
    assert!(output.contains("last"));
    assert!(output.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES);
}

#[test]
fn oversized_first_and_last_messages_are_shortened() {
    let large = "x".repeat(OTLP_MAX_ATTRIBUTE_VALUE_BYTES);
    let payload = serde_json::to_string(&vec![
        serde_json::json!({"role":"user", "content":large}),
        serde_json::json!({"role":"assistant", "content":"last"}),
    ])
    .unwrap();
    let output = truncate_payload(&payload);
    assert!(output.contains("truncated"));
    assert!(output.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES);
}
