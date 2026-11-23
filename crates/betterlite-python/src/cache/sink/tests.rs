#![cfg(test)]

use betterlite_redact::structured::Payload;

use super::payload_text;

#[test]
fn a_string_message_logs_without_json_quotes() {
    assert_eq!(
        payload_text(&Payload::String("LiteLLM Cache: exception".to_owned())),
        "LiteLLM Cache: exception"
    );
}

#[test]
fn any_other_payload_keeps_its_json_text() {
    assert_eq!(payload_text(&Payload::Bool(true)), "true");
    assert_eq!(payload_text(&Payload::Null), "null");
}
