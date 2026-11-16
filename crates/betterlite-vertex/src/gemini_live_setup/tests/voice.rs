#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::gemini_live_setup::voice::{SpeechSelection, speech_config};

use super::support::{as_json, mapped, parse, with_warnings};

#[rstest]
#[case::alloy("alloy")]
#[case::upper_verse("VERSE")]
#[case::mixed_echo("Echo")]
#[case::shimmer("Shimmer")]
fn stock_openai_voices_are_dropped(#[case] voice: &str) {
    assert_eq!(
        speech_config(voice).expect("the voice maps"),
        SpeechSelection::Dropped
    );
}

#[rstest]
#[case::puck(
    "Puck",
    r#"{"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}}"#
)]
#[case::empty("", r#"{"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": ""}}}"#)]
#[case::near_miss(
    "Alloys",
    r#"{"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Alloys"}}}"#
)]
fn custom_voices_are_forwarded_verbatim(#[case] voice: &str, #[case] expected: &str) {
    let selection = speech_config(voice).expect("the voice maps");
    let SpeechSelection::Voice(config) = selection else {
        panic!("{voice} is not a stock voice");
    };
    assert_eq!(as_json(&config), parse::<Json>(expected));
}

#[test]
fn a_stock_voice_leaves_the_setup_without_a_speech_config() {
    assert_eq!(
        as_json(&mapped(r#"{"voice": "Alloy"}"#)),
        parse::<Json>("{}")
    );
}

#[test]
fn a_dropped_voice_logs_the_source_warning() {
    let logged = with_warnings(|| {
        mapped(r#"{"voice": "Alloy"}"#);
    });
    assert!(logged.contains("WARN"));
    assert!(logged.contains("Alloy is an OpenAI voice with no Gemini equivalent"));
}

#[test]
fn a_custom_voice_logs_no_warning() {
    let logged = with_warnings(|| {
        mapped(r#"{"voice": "Puck"}"#);
    });
    assert!(!logged.contains("WARN"));
}
