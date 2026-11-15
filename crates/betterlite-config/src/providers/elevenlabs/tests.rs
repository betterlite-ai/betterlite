#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::types::LiteLlmParams;

const DEFAULT_API_BASE: &str = "https://api.elevenlabs.io";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn elevenlabs_speech_settings_default_to_an_unset_key() {
    let (api_key, api_base) = LiteLlmParams::elevenlabs_speech_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn elevenlabs_speech_settings_read_the_environment() {
    let (api_key, api_base) = LiteLlmParams::elevenlabs_speech_settings(&lookup(&[
        ("ELEVENLABS_API_KEY", "server-key"),
        ("ELEVENLABS_API_BASE", "https://proxy.test"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test");
}

#[rstest]
#[case::empty_base_is_unset("ELEVENLABS_API_BASE", "", None, DEFAULT_API_BASE)]
#[case::empty_key_is_kept("ELEVENLABS_API_KEY", "", Some(""), DEFAULT_API_BASE)]
#[case::true_key_is_unset("ELEVENLABS_API_KEY", "true", None, DEFAULT_API_BASE)]
#[case::false_base_is_unset("ELEVENLABS_API_BASE", " False ", None, DEFAULT_API_BASE)]
#[case::other_text_key_is_kept("ELEVENLABS_API_KEY", "yes", Some("yes"), DEFAULT_API_BASE)]
#[case::other_text_base_is_kept("ELEVENLABS_API_BASE", "no", None, "no")]
fn elevenlabs_speech_settings_treat_empty_values(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let (api_key, api_base) = LiteLlmParams::elevenlabs_speech_settings(&lookup(&[(name, value)]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
    assert_eq!(api_base, expected_base);
}
