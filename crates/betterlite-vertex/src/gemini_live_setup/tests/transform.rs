#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use crate::gemini_live_setup::decode::decode_gemini_live_setup;

use super::support::{as_json, parse, transform, with_warnings};

#[test]
fn the_first_update_sends_one_setup() {
    let messages = transform(
        r#"{"instructions": "Be brief.", "voice": "Puck"}"#,
        "gemini-2.5-flash",
        None,
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(
        as_json(&decode_gemini_live_setup(&messages[0]).expect("the setup decodes")),
        parse::<Json>(
            r#"{
                "model": "models/gemini-2.5-flash",
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}}
                },
                "systemInstruction": {"role": "user", "parts": [{"text": "Be brief."}]},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn the_first_update_defaults_the_modality_and_transcription() {
    let messages = transform("{}", "gemini-2.5-flash", None);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        as_json(&decode_gemini_live_setup(&messages[0]).expect("the setup decodes")),
        parse::<Json>(
            r#"{
                "model": "models/gemini-2.5-flash",
                "generationConfig": {"responseModalities": ["AUDIO"]},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn an_explicit_empty_modality_list_is_kept() {
    let messages = transform(r#"{"modalities": []}"#, "gemini-2.5-flash", None);
    let setup = decode_gemini_live_setup(&messages[0]).expect("the setup decodes");
    assert_eq!(
        as_json(&setup.generation_config.response_modalities),
        parse::<Json>("[]")
    );
}

#[test]
fn the_first_update_coerces_the_modalities_for_the_model() {
    let messages = transform(
        r#"{"modalities": ["text", "audio"]}"#,
        "gemini-2.5-flash-native-audio-preview-09-2025",
        None,
    );
    let setup = decode_gemini_live_setup(&messages[0]).expect("the setup decodes");
    assert_eq!(
        as_json(&setup.generation_config.response_modalities),
        parse::<Json>(r#"["AUDIO"]"#)
    );
}

#[test]
fn a_later_update_sends_nothing() {
    assert!(
        transform(
            r#"{"instructions": "late"}"#,
            "gemini-2.5-flash",
            Some("stored")
        )
        .is_empty()
    );
}

#[test]
fn a_later_update_is_dropped_even_with_new_tools() {
    let messages = transform(
        r#"{"tools": [{"type": "function", "function": {"name": "late"}}]}"#,
        "gemini-2.5-flash",
        Some("stored"),
    );
    assert!(messages.is_empty());
}

#[test]
fn an_empty_request_string_counts_as_a_later_update() {
    assert!(transform("{}", "gemini-2.5-flash", Some("")).is_empty());
}

#[test]
fn a_later_guardrail_update_logs_the_drop_warning() {
    let logged = with_warnings(|| {
        transform(
            r#"{"turn_detection": {"type": "semantic_vad", "create_response": false}}"#,
            "gemini-2.5-flash",
            Some("stored"),
        );
    });
    assert!(logged.contains("WARN"));
    assert!(logged.contains("rejects a second setup message"));
}

#[test]
fn a_later_ga_guardrail_update_logs_the_drop_warning() {
    let logged = with_warnings(|| {
        transform(
            r#"{"audio": {"input": {"turn_detection": {"type": "server_vad", "create_response": false}}}}"#,
            "gemini-2.5-flash",
            Some("stored"),
        );
    });
    assert!(logged.contains("rejects a second setup message"));
}

#[test]
fn a_later_update_without_a_guardrail_logs_no_warning() {
    let logged = with_warnings(|| {
        transform(
            r#"{"turn_detection": {"type": "server_vad"}}"#,
            "gemini-2.5-flash",
            Some("stored"),
        );
    });
    assert!(!logged.contains("WARN"));
}

#[test]
fn a_later_semantic_vad_update_without_the_override_logs_no_warning() {
    let logged = with_warnings(|| {
        transform(
            r#"{"turn_detection": {"type": "semantic_vad"}}"#,
            "gemini-2.5-flash",
            Some("stored"),
        );
    });
    assert!(!logged.contains("WARN"));
}
