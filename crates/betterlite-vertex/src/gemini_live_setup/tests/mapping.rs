#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;

use crate::gemini_live_setup::setup::map_session;
use crate::gemini_tools::assembly::map_tools;

use crate::gemini_live_setup::session::SessionUpdate;

use super::support::{as_json, mapped, parse};

const TOOLS: &str = r#"[{"type": "function", "function": {"name": "lookup", "description": "Find a city", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}}]"#;

#[test]
fn every_supported_option_maps_to_its_wire_field() {
    assert_eq!(
        as_json(&mapped(
            r#"{
                "instructions": "Be brief.",
                "temperature": 0.4,
                "max_response_output_tokens": 256,
                "modalities": ["text", "audio"],
                "input_audio_transcription": {"model": "whisper-1"},
                "turn_detection": {"type": "server_vad", "prefix_padding_ms": 20, "silence_duration_ms": 500},
                "voice": "Puck"
            }"#
        )),
        parse::<Json>(
            r#"{
                "generationConfig": {
                    "temperature": 0.4,
                    "maxOutputTokens": 256,
                    "responseModalities": ["TEXT", "AUDIO"],
                    "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}}
                },
                "systemInstruction": {"role": "user", "parts": [{"text": "Be brief."}]},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": false, "prefixPaddingMs": 20, "silenceDurationMs": 500}},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn an_empty_session_maps_to_an_empty_setup() {
    assert_eq!(as_json(&mapped("{}")), parse::<Json>("{}"));
}

#[test]
fn unknown_session_keys_and_null_options_are_ignored() {
    assert_eq!(
        as_json(&mapped(
            r#"{"modalities": null, "voice": null, "unexpected": {"deep": true}}"#
        )),
        parse::<Json>("{}")
    );
}

#[test]
fn ga_audio_slots_fill_the_absent_flat_keys() {
    assert_eq!(
        as_json(&mapped(
            r#"{
                "output_modalities": ["text", "audio"],
                "audio": {
                    "input": {
                        "transcription": {"model": "whisper-1"},
                        "turn_detection": {"type": "server_vad", "create_response": false}
                    },
                    "output": {"voice": "Charon"}
                }
            }"#
        )),
        parse::<Json>(
            r#"{
                "generationConfig": {
                    "responseModalities": ["TEXT", "AUDIO"],
                    "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Charon"}}}
                },
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true}},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn flat_keys_win_over_the_ga_slots() {
    assert_eq!(
        as_json(&mapped(
            r#"{
                "modalities": ["audio"],
                "output_modalities": ["text"],
                "input_audio_transcription": {"model": "flat"},
                "audio": {
                    "input": {
                        "transcription": {"model": "nested"},
                        "turn_detection": {"type": "server_vad", "create_response": false}
                    }
                },
                "turn_detection": {"type": "server_vad"}
            }"#
        )),
        parse::<Json>(
            r#"{
                "generationConfig": {"responseModalities": ["AUDIO"]},
                "realtimeInputConfig": {"automaticActivityDetection": {"disabled": false}},
                "inputAudioTranscription": {}
            }"#
        )
    );
}

#[test]
fn max_response_output_tokens_accepts_counts_and_inf() {
    assert_eq!(
        as_json(&mapped(r#"{"max_response_output_tokens": -1}"#)),
        parse::<Json>(r#"{"generationConfig": {"maxOutputTokens": -1}}"#)
    );
    assert_eq!(
        as_json(&mapped(r#"{"max_response_output_tokens": "inf"}"#)),
        parse::<Json>(r#"{"generationConfig": {"maxOutputTokens": "inf"}}"#)
    );
}

#[test]
fn tools_map_to_the_top_level_of_the_setup() {
    let update = parse::<SessionUpdate>(&format!(r#"{{"tools": {TOOLS}}}"#));
    let setup = map_session(&update, &SchemaSettings::default()).expect("the session maps");
    let expected = map_tools(
        update.tools.as_deref().expect("the session has tools"),
        false,
        None,
        &SchemaSettings::default(),
    )
    .expect("the tools map");
    assert_eq!(setup.tools, Some(expected.tools));
    assert_eq!(setup.tool_config, expected.tool_config);
    assert!(setup.tools.as_ref().is_some_and(|tools| !tools.is_empty()));
    assert_eq!(as_json(&setup.generation_config), parse::<Json>("{}"));
}

#[test]
fn maps_coordinates_become_the_top_level_tool_config() {
    let update =
        parse::<SessionUpdate>(r#"{"tools": [{"googleMaps": {"latitude": 1, "longitude": 2}}]}"#);
    let setup = map_session(&update, &SchemaSettings::default()).expect("the session maps");
    let expected = map_tools(
        update.tools.as_deref().expect("the session has tools"),
        false,
        None,
        &SchemaSettings::default(),
    )
    .expect("the tools map");
    assert_eq!(setup.tool_config, expected.tool_config);
    assert_eq!(
        as_json(&setup.tool_config),
        parse::<Json>(r#"{"retrievalConfig": {"latLng": {"latitude": 1, "longitude": 2}}}"#)
    );
}
