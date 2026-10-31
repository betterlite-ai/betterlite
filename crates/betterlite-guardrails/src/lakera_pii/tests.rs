#![cfg(test)]

use std::collections::BTreeMap;

use super::masking::{blocked_error, is_only_pii_violation, mask_pii_in_messages};
use super::wire::{BreakdownItem, LakeraResponse, Message, PayloadItem};

fn message(content: &str) -> Message {
    Message {
        content: Some(content.to_owned()),
        extra: BTreeMap::new(),
    }
}

fn payload_item(message_id: u64, start: u64, end: u64, detector_type: &str) -> PayloadItem {
    PayloadItem {
        message_id: Some(message_id),
        start: Some(start),
        end: Some(end),
        detector_type: Some(detector_type.to_owned()),
        extra: BTreeMap::new(),
    }
}

fn breakdown_item(detected: bool, detector_type: &str) -> BreakdownItem {
    BreakdownItem {
        detected: Some(detected),
        detector_type: Some(detector_type.to_owned()),
        extra: BTreeMap::new(),
    }
}

#[test]
fn lakera_pii_behaviors() {
    let response = LakeraResponse {
        flagged: None,
        payload: None,
        breakdown: Some(vec![breakdown_item(true, "pii/email")]),
        extra: BTreeMap::new(),
    };
    assert!(is_only_pii_violation(Some(&response)));
    let blocked = blocked_error(Some(&response));
    assert_eq!(blocked.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[test]
fn lakera_pii_masking() {
    let messages = vec![message("email a@example.com")];
    let response = LakeraResponse {
        flagged: None,
        payload: Some(vec![payload_item(0, 6, 19, "pii/email")]),
        breakdown: None,
        extra: BTreeMap::new(),
    };
    let mut counts = BTreeMap::new();
    let masked = mask_pii_in_messages(&messages, Some(&response), &mut counts);
    assert_eq!(masked[0].content.as_deref(), Some("email [MASKED EMAIL]"));
    assert_eq!(counts["EMAIL"], 1);
}

#[test]
fn lakera_pii_masking_uses_character_offsets() {
    let messages = vec![message("\u{e9} a@example.com")];
    let response = LakeraResponse {
        flagged: None,
        payload: Some(vec![payload_item(0, 2, 15, "pii/email")]),
        breakdown: None,
        extra: BTreeMap::new(),
    };
    let mut counts = BTreeMap::new();
    let masked = mask_pii_in_messages(&messages, Some(&response), &mut counts);
    assert_eq!(masked[0].content.as_deref(), Some("\u{e9} [MASKED EMAIL]"));
}

#[test]
fn lakera_pii_masking_empty_detector_segment_falls_back_to_pii() {
    let messages = vec![message("secret value")];
    let response = LakeraResponse {
        flagged: None,
        payload: Some(vec![payload_item(0, 0, 6, "pii/")]),
        breakdown: None,
        extra: BTreeMap::new(),
    };
    let mut counts = BTreeMap::new();
    let masked = mask_pii_in_messages(&messages, Some(&response), &mut counts);
    assert_eq!(masked[0].content.as_deref(), Some("[MASKED PII] value"));
    assert_eq!(counts["PII"], 1);
}
