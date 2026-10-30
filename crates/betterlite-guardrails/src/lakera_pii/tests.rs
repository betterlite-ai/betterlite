#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::json;

use super::{blocked_exception, is_only_pii_violation, mask_pii_in_messages};

#[test]
fn lakera_pii_behaviors() {
    let response = json!({"breakdown":[{"detected":true,"detector_type":"pii/email"}]});
    assert!(is_only_pii_violation(Some(&response)));
    assert_eq!(blocked_exception(Some(&response))["status_code"], 400);
}

#[test]
fn lakera_pii_masking() {
    let messages = vec![json!({"role":"user","content":"email a@example.com"})];
    let response =
        json!({"payload":[{"message_id":0,"start":6,"end":19,"detector_type":"pii/email"}]});
    let mut counts = BTreeMap::new();
    assert_eq!(
        mask_pii_in_messages(&messages, Some(&response), &mut counts)[0]["content"],
        "email [MASKED EMAIL]"
    );
    assert_eq!(counts["EMAIL"], 1);
}

#[test]
fn lakera_pii_masking_uses_character_offsets() {
    let messages = vec![json!({"role":"user","content":"é a@example.com"})];
    let response =
        json!({"payload":[{"message_id":0,"start":2,"end":15,"detector_type":"pii/email"}]});
    let mut counts = BTreeMap::new();
    assert_eq!(
        mask_pii_in_messages(&messages, Some(&response), &mut counts)[0]["content"],
        "é [MASKED EMAIL]"
    );
}
