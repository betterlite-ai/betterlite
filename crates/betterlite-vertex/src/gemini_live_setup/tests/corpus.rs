#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::error::LiveSetupError;

use super::support::{as_json, parse};

const SEEDS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fuzz/corpus/gemini_live_setup/"
);

fn seed(name: &str) -> String {
    fs::read_to_string(format!("{SEEDS}{name}")).expect("the seed is readable text")
}

#[rstest]
fn replays_the_gemini_live_setup_corpus(#[files("fuzz/corpus/gemini_live_setup/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the seed is readable");
    let Ok(body) = from_utf8(&bytes) else {
        return;
    };
    drop(decode_gemini_live_setup(body));
}

#[rstest]
#[case::setup(
    "valid-setup.json",
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
)]
#[case::missing_setup("valid-missing-setup.json", "{}")]
#[case::null_setup("valid-null-setup.json", "{}")]
#[case::empty_setup("valid-empty-setup.json", "{}")]
#[case::nested_vad(
    "valid-nested-vad.json",
    r#"{
        "generationConfig": {"responseModalities": ["TEXT"]},
        "realtimeInputConfig": {"automaticActivityDetection": {"disabled": true, "silenceDurationMs": true}}
    }"#
)]
fn decodes_the_valid_gemini_live_setup_seeds(#[case] name: &str, #[case] expected: &str) {
    let setup = decode_gemini_live_setup(&seed(name)).expect("the setup decodes");
    assert_eq!(as_json(&setup), parse::<Json>(expected));
}

#[rstest]
#[case::field_type("invalid-field-type.json")]
#[case::token_limit("invalid-token-limit.json")]
fn rejects_the_invalid_field_gemini_live_setup_seeds(#[case] name: &str) {
    assert!(matches!(
        decode_gemini_live_setup(&seed(name)),
        Err(LiveSetupError::InvalidField { .. })
    ));
}

#[rstest]
#[case::trailing_data("invalid-trailing-data.json")]
#[case::malformed("malformed-json.json")]
fn rejects_the_invalid_json_gemini_live_setup_seeds(#[case] name: &str) {
    assert!(matches!(
        decode_gemini_live_setup(&seed(name)),
        Err(LiveSetupError::InvalidJson(_))
    ));
}
