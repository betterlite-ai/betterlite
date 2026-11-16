#![cfg(test)]

use crate::gemini_live_setup::session::normalize_session;

use super::support::{as_json, json, session};

#[test]
fn output_modalities_fill_an_absent_flat_key() {
    let normalized = normalize_session(&session(r#"{"output_modalities": ["text"]}"#));
    assert_eq!(normalized.modalities, Some(vec!["text".to_owned()]));
}

#[test]
fn present_flat_modalities_keep_their_value() {
    let normalized = normalize_session(&session(
        r#"{"modalities": ["audio"], "output_modalities": ["text"]}"#,
    ));
    assert_eq!(normalized.modalities, Some(vec!["audio".to_owned()]));
}

#[test]
fn the_nested_transcription_fills_only_an_absent_flat_key() {
    let nested = normalize_session(&session(
        r#"{"audio": {"input": {"transcription": {"model": "nested"}}}}"#,
    ));
    assert_eq!(
        as_json(&nested.input_audio_transcription),
        json(r#"{"model": "nested"}"#)
    );
    let flat = normalize_session(&session(
        r#"{"input_audio_transcription": {"model": "flat"}, "audio": {"input": {"transcription": {"model": "nested"}}}}"#,
    ));
    assert_eq!(
        as_json(&flat.input_audio_transcription),
        json(r#"{"model": "flat"}"#)
    );
}

#[test]
fn the_nested_turn_detection_fills_only_an_absent_flat_key() {
    let nested = normalize_session(&session(
        r#"{"audio": {"input": {"turn_detection": {"type": "server_vad", "create_response": false}}}}"#,
    ));
    assert_eq!(
        nested
            .turn_detection
            .as_ref()
            .and_then(|detection| detection.create_response),
        Some(false)
    );
    let flat = normalize_session(&session(
        r#"{"turn_detection": {"type": "server_vad"}, "audio": {"input": {"turn_detection": {"type": "server_vad", "create_response": false}}}}"#,
    ));
    assert_eq!(
        flat.turn_detection
            .as_ref()
            .and_then(|detection| detection.create_response),
        None
    );
}

#[test]
fn a_truthy_nested_voice_replaces_the_flat_voice() {
    let normalized = normalize_session(&session(
        r#"{"voice": "Puck", "audio": {"output": {"voice": "Charon"}}}"#,
    ));
    assert_eq!(normalized.voice.as_deref(), Some("Charon"));
}

#[test]
fn an_empty_nested_voice_keeps_the_flat_voice() {
    let normalized = normalize_session(&session(
        r#"{"voice": "Puck", "audio": {"output": {"voice": ""}}}"#,
    ));
    assert_eq!(normalized.voice.as_deref(), Some("Puck"));
}

#[test]
fn a_nested_voice_without_a_flat_voice_is_used() {
    let normalized = normalize_session(&session(r#"{"audio": {"output": {"voice": "Charon"}}}"#));
    assert_eq!(normalized.voice.as_deref(), Some("Charon"));
}
