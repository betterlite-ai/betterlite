#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::error::LiveSetupError;

use super::support::{as_json, json};

#[rstest]
fn replays_the_gemini_live_setup_corpus(#[files("fuzz/corpus/gemini_live_setup/*")] path: PathBuf) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the seed has a name");
    let body = fs::read_to_string(&path).expect("the seed is readable text");
    match name {
        "valid-setup.json" => {
            let setup = decode_gemini_live_setup(&body).expect("the setup decodes");
            assert_eq!(
                as_json(&setup),
                json(
                    r#"{
                        "model": "models/gemini-2.5-flash",
                        "generationConfig": {
                            "responseModalities": ["AUDIO"],
                            "maxOutputTokens": 256,
                            "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}}
                        },
                        "realtimeInputConfig": {"automaticActivityDetection": {"disabled": false, "prefixPaddingMs": 20}},
                        "inputAudioTranscription": {}
                    }"#
                )
            );
        }
        "valid-missing-setup.json" | "valid-null-setup.json" | "valid-empty-setup.json" => {
            assert_eq!(
                as_json(&decode_gemini_live_setup(&body).expect("the setup decodes")),
                json("{}")
            );
        }
        "valid-nested-vad.json" => {
            let setup = decode_gemini_live_setup(&body).expect("the setup decodes");
            assert_eq!(
                as_json(&setup.generation_config.response_modalities),
                json(r#"["TEXT"]"#)
            );
            assert_eq!(
                as_json(&setup.realtime_input_config),
                json(
                    r#"{"automaticActivityDetection": {"disabled": true, "silenceDurationMs": true}}"#
                )
            );
        }
        "invalid-field-type.json" | "invalid-token-limit.json" => {
            assert!(matches!(
                decode_gemini_live_setup(&body),
                Err(LiveSetupError::InvalidField { .. })
            ));
        }
        "invalid-trailing-data.json" | "malformed-json.json" => {
            assert!(matches!(
                decode_gemini_live_setup(&body),
                Err(LiveSetupError::InvalidJson(_))
            ));
        }
        other => panic!("unexpected seed {other}"),
    }
}
