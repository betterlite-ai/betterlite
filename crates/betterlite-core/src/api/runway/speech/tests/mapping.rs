use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use rstest::rstest;
use serde_json::json;

use super::super::RunwaySpeechConfig;
use super::super::mapping::{VOICE_KEY, map_openai_params};
use super::support::{preset_object, settings, text_params};
use crate::api::speech_call::operations::{SpeechConfigOperation, SpeechMappingOptions};

#[rstest]
#[case::alloy("alloy", "Maya")]
#[case::echo("echo", "James")]
#[case::fable("fable", "Bernard")]
#[case::onyx("onyx", "Vincent")]
#[case::nova("nova", "Serene")]
#[case::shimmer("shimmer", "Ella")]
fn mapped_openai_voices_become_runway_presets(#[case] name: &str, #[case] preset: &str) {
    let voice = SpeechVoice::Name(name.to_owned());
    let (mapped, params) = map_openai_params(Some(&voice));
    assert_eq!(mapped, None);
    assert_eq!(params.len(), 1);
    assert_eq!(params.get(VOICE_KEY), Some(&json!(preset_object(preset))));
}

#[test]
fn an_unknown_voice_is_a_literal_preset_id() {
    let voice = SpeechVoice::Name(String::from("Cosmo"));
    let (_, params) = map_openai_params(Some(&voice));
    assert_eq!(params.get(VOICE_KEY), Some(&json!(preset_object("Cosmo"))));
}

#[test]
fn a_mapping_voice_passes_through() {
    let mut fields = Object::new();
    fields.insert(String::from("type"), json!("custom-preset"));
    fields.insert(String::from("presetId"), json!("Maya"));
    fields.insert(String::from("weight"), json!(2_i32));
    let voice = SpeechVoice::Options(fields.clone());
    let (_, params) = map_openai_params(Some(&voice));
    assert_eq!(params.get(VOICE_KEY), Some(&json!(fields)));
}

#[test]
fn an_empty_mapping_remains_the_selected_voice() {
    let voice = SpeechVoice::Options(Object::new());
    let (_, params) = map_openai_params(Some(&voice));
    assert_eq!(params.get(VOICE_KEY), Some(&json!(Object::new())));
}

#[test]
fn a_missing_voice_maps_to_nothing() {
    let (mapped, params) = map_openai_params(None);
    assert_eq!(mapped, None);
    assert!(params.is_empty());
}

#[test]
fn the_config_mapping_ignores_received_options_and_drop_params() {
    let config = RunwaySpeechConfig {
        settings: settings(&[]),
    };
    let options = SpeechMappingOptions {
        voice: Some(SpeechVoice::Name(String::from("alloy"))),
        drop_params: true,
        kwargs: Object::new(),
    };
    let (voice, mapped) = SpeechConfigOperation::map_openai_params(
        &config,
        "eleven_multilingual_v2",
        text_params(&[("speed", "2"), ("response_format", "mp3")]),
        &options,
    );
    assert_eq!(voice, None);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped.get(VOICE_KEY), Some(&json!(preset_object("Maya"))));
    assert!(!mapped.contains_key("speed"));
    assert!(!mapped.contains_key("response_format"));
}

#[test]
fn the_config_mapping_without_a_voice_maps_nothing() {
    let config = RunwaySpeechConfig {
        settings: settings(&[]),
    };
    let options = SpeechMappingOptions {
        voice: None,
        drop_params: false,
        kwargs: Object::new(),
    };
    let (voice, mapped) = SpeechConfigOperation::map_openai_params(
        &config,
        "eleven_multilingual_v2",
        text_params(&[("speed", "2")]),
        &options,
    );
    assert_eq!(voice, None);
    assert!(mapped.is_empty());
}
