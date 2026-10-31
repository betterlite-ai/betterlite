#![cfg(test)]

use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::masking::{blocked_error, is_only_pii_violation, mask_pii_in_messages};
use super::wire::{BreakdownItem, LakeraResponse, Message, PayloadItem};

fn message(content: &str) -> Message {
    Message {
        content: Some(
            RawValue::from_string(serde_json::to_string(content).expect("string")).expect("raw"),
        ),
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
    assert_eq!(
        masked[0]
            .content
            .as_ref()
            .and_then(|content| serde_json::from_str::<String>(content.get()).ok()),
        Some("email [MASKED EMAIL]".to_owned())
    );
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
    assert_eq!(
        masked[0]
            .content
            .as_ref()
            .and_then(|content| serde_json::from_str::<String>(content.get()).ok()),
        Some("\u{e9} [MASKED EMAIL]".to_owned())
    );
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
    assert_eq!(
        masked[0]
            .content
            .as_ref()
            .and_then(|content| serde_json::from_str::<String>(content.get()).ok()),
        Some("[MASKED PII] value".to_owned())
    );
    assert_eq!(counts["PII"], 1);
}

#[test]
fn lakera_pii_accepts_multimodal_content_and_replays_corpus() {
    let messages: Vec<Message> =
        serde_json::from_str(r#"[{"content":[{"type":"text","text":"email"}]},{"content":null}]"#)
            .expect("messages");
    let response = LakeraResponse {
        flagged: None,
        payload: None,
        breakdown: None,
        extra: BTreeMap::new(),
    };
    let mut counts = BTreeMap::new();
    let unchanged = mask_pii_in_messages(&messages, Some(&response), &mut counts);
    assert_eq!(
        serde_json::to_value(unchanged).expect("encoded"),
        serde_json::to_value(messages).expect("encoded")
    );
    replay_lakera_corpus();
}

fn replay_lakera_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/lakera_wire");
    for entry in fs::read_dir(corpus).expect("corpus") {
        let bytes = fs::read(entry.expect("entry").path()).expect("seed");
        let _messages: Vec<Message> = serde_json::from_slice(&bytes).expect("messages");
    }
}

#[test]
fn lakera_pii_omitted_start_defaults_to_zero() {
    let messages = vec![message("secret value")];
    let response: LakeraResponse = serde_json::from_str(
        r#"{"payload":[{"message_id":0,"end":6,"detector_type":"pii/name"}]}"#,
    )
    .expect("response");
    let mut counts = BTreeMap::new();
    let masked = mask_pii_in_messages(&messages, Some(&response), &mut counts);
    assert_eq!(
        masked[0]
            .content
            .as_ref()
            .and_then(|content| serde_json::from_str::<String>(content.get()).ok()),
        Some("[MASKED NAME] value".to_owned())
    );
}
