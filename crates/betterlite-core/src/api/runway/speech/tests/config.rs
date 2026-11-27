use std::collections::BTreeMap;

use super::super::RunwaySpeechConfig;
use super::super::SUPPORTED_OPENAI_PARAMS;

#[test]
fn the_selector_reads_the_runway_environment() {
    let config = RunwaySpeechConfig::for_model(
        "eleven_multilingual_v2",
        &BTreeMap::from([
            (
                String::from("RUNWAYML_API_BASE"),
                String::from("https://other.test"),
            ),
            (
                String::from("RUNWAYML_DEFAULT_API_VERSION"),
                String::from("2025-01-01"),
            ),
            (String::from("RUNWAYML_POLLING_TIMEOUT"), String::from("30")),
        ]),
    )
    .unwrap();
    assert_eq!(config.settings.api_base, "https://other.test");
    assert_eq!(config.settings.api_version, "2025-01-01");
    assert_eq!(config.settings.polling_timeout_seconds, 30);
}

#[test]
fn the_selector_needs_no_credentials() {
    let config = RunwaySpeechConfig::for_model("eleven_multilingual_v2", &BTreeMap::new()).unwrap();
    assert_eq!(config.settings.api_base, "https://api.dev.runwayml.com");
    assert_eq!(config.settings.api_version, "2024-11-06");
    assert_eq!(config.settings.polling_timeout_seconds, 600);
    assert!(config.settings.api_secret.is_none());
    assert!(config.settings.api_key.is_none());
}

#[test]
fn the_selector_answers_every_model_name() {
    let first = RunwaySpeechConfig::for_model("tts-1", &BTreeMap::new()).unwrap();
    let second = RunwaySpeechConfig::for_model("eleven_multilingual_v2", &BTreeMap::new()).unwrap();
    assert_eq!(first.settings.api_base, second.settings.api_base);
}

#[test]
fn only_voice_is_a_supported_openai_param() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["voice"]);
}
