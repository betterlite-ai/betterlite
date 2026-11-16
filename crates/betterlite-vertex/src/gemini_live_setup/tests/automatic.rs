#![cfg(test)]

use crate::gemini_live_setup::decode::decode_gemini_live_setup;

use super::support::{as_json, automatic, json};

#[test]
fn the_automatic_setup_uses_the_default_modality_and_input_transcription() {
    let request = automatic("gemini-2.5-flash", false).expect("the request builds");
    assert_eq!(
        as_json(&decode_gemini_live_setup(&request).expect("the setup decodes")),
        json(
            r#"{
                "model": "models/gemini-2.5-flash",
                "generationConfig": {"responseModalities": ["AUDIO"]},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn a_text_only_model_gets_text_automatically() {
    let request = automatic("gemini-3.5-transcribe-live", false).expect("the request builds");
    let setup = decode_gemini_live_setup(&request).expect("the setup decodes");
    assert_eq!(
        as_json(&setup.generation_config.response_modalities),
        json(r#"["TEXT"]"#)
    );
}

#[test]
fn a_deferred_session_skips_the_automatic_request() {
    assert!(automatic("gemini-2.5-flash", true).is_none());
}

#[test]
fn the_automatic_setup_carries_no_unrequested_fields() {
    let request = automatic("gemini-2.5-flash", false).expect("the request builds");
    let setup = decode_gemini_live_setup(&request).expect("the setup decodes");
    assert!(setup.system_instruction.is_none());
    assert!(setup.tools.is_none());
    assert!(setup.tool_config.is_none());
    assert!(setup.realtime_input_config.is_none());
    assert!(setup.session_resumption.is_none());
    assert!(setup.context_window_compression.is_none());
    assert!(setup.output_audio_transcription.is_none());
    assert!(setup.generation_config.temperature.is_none());
    assert!(setup.generation_config.max_output_tokens.is_none());
    assert!(setup.generation_config.speech_config.is_none());
}
