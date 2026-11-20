#![cfg(test)]

use betterlite_core::api::realtime_contracts::close::backend_close_from;
use serde_json::json;

use super::input::{Input, MAX_FRAMES, MAX_TEXT_BYTES};
use super::transport::{closed, recorded};

#[test]
fn input_rejects_more_frames_than_the_cap() {
    let mut input = Input::default();
    input.frames = vec![String::from("{}"); MAX_FRAMES + 1];
    assert!(input.check().unwrap_err().contains("limits"));
}

#[test]
fn input_rejects_a_frame_over_the_byte_cap() {
    let mut input = Input::default();
    input.frames = vec![String::from("x").repeat(MAX_TEXT_BYTES + 1)];
    assert!(input.check().unwrap_err().contains("limits"));
}

#[test]
fn recorded_keeps_a_json_frame_as_json() {
    assert_eq!(
        recorded(r#"{"type":"session.created"}"#),
        json!({"kind": "json", "value": {"type": "session.created"}})
    );
}

#[test]
fn recorded_labels_a_plain_text_frame() {
    assert_eq!(
        recorded("not json"),
        json!({"kind": "text", "text": "not json"})
    );
}

#[test]
fn a_missing_close_frame_maps_to_1006_and_its_reason() {
    let close = backend_close_from(&closed(None));
    assert_eq!(close.code, 1006);
    assert_eq!(close.reason, "no close frame received or sent");
}
