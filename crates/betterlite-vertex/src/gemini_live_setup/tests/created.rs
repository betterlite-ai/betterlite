#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use rstest::rstest;

use crate::gemini_live_setup::created::CreatedEvent;

use super::support::{as_json, created, parse};

const EVENT_ID: &str = "00000000-0000-4000-8000-000000000001";

#[test]
fn a_string_instruction_is_copied_and_the_model_is_stripped() {
    let request = r#"{
        "setup": {
            "model": "vertex_ai/models/live-model",
            "generationConfig": {"responseModalities": ["AUDIO", "TEXT"]},
            "systemInstruction": "Be brief."
        }
    }"#;
    assert_eq!(
        as_json(&created(
            "argument-model",
            "session-1",
            Some(request),
            EVENT_ID
        )),
        parse::<Json>(
            r#"{
                "type": "session.created",
                "session": {
                    "id": "session-1",
                    "modalities": ["audio", "text"],
                    "instructions": "Be brief.",
                    "model": "live-model"
                },
                "event_id": "00000000-0000-4000-8000-000000000001"
            }"#
        )
    );
}

#[test]
fn a_content_instruction_is_not_copied() {
    let request = r#"{
        "setup": {
            "model": "models/live-model",
            "generationConfig": {"responseModalities": ["AUDIO"]},
            "systemInstruction": {"role": "user", "parts": [{"text": "ignored"}]}
        }
    }"#;
    let event = created("argument-model", "session-1", Some(request), EVENT_ID);
    assert!(event.session.instructions.is_none());
    assert_eq!(event.session.model, "live-model");
    assert_eq!(event.session.modalities, vec!["audio".to_owned()]);
}

#[test]
fn the_argument_model_is_used_without_a_setup_model() {
    assert_eq!(
        created(
            "argument-model",
            "session-1",
            Some(r#"{"setup": {}}"#),
            EVENT_ID
        )
        .session
        .model,
        "argument-model"
    );
    assert_eq!(
        created("argument-model", "session-1", None, EVENT_ID)
            .session
            .model,
        "argument-model"
    );
    assert_eq!(
        created("argument-model", "session-1", Some(""), EVENT_ID)
            .session
            .model,
        "argument-model"
    );
}

#[test]
fn the_created_event_defaults_to_audio_modalities() {
    let event = created("gemini-2.5-flash", "logging-session", None, EVENT_ID);
    assert_eq!(event.event_type, CreatedEvent::SessionCreated);
    assert_eq!(event.session.id, "logging-session");
    assert_eq!(event.session.modalities, vec!["audio".to_owned()]);
    assert_eq!(event.event_id, EVENT_ID);
    assert!(event.session.instructions.is_none());
}

#[rstest]
#[case::vertex_path("vertex_ai/models/gemini-live", "gemini-live")]
#[case::studio_path("models/gemini-live", "gemini-live")]
#[case::bare("gemini-live", "gemini-live")]
#[case::last_models_segment("a/models/b/models/c", "c")]
#[case::path_without_models("vertex_ai/gemini-live", "vertex_ai/gemini-live")]
fn the_model_keeps_only_the_bare_name(#[case] setup_model: &str, #[case] expected: &str) {
    let request = format!(r#"{{"setup": {{"model": "{setup_model}"}}}}"#);
    let event = created("argument", "session-1", Some(&request), EVENT_ID);
    assert_eq!(event.session.model, expected);
}
