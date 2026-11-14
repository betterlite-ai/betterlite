#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://api.deepgram.com/v1";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn deepgram_transcription_settings_default_to_the_hosted_base_and_no_key() {
    let (api_key, api_base) = Sections::deepgram_transcript_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn deepgram_transcription_settings_read_the_environment() {
    let (api_key, api_base) = Sections::deepgram_transcript_settings(&lookup(&[
        ("DEEPGRAM_API_KEY", "server-key"),
        ("DEEPGRAM_API_BASE", "https://proxy.test/v1"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test/v1");
}

#[test]
fn deepgram_transcription_settings_treat_empty_variables_as_unset() {
    let (api_key, api_base) = Sections::deepgram_transcript_settings(&lookup(&[
        ("DEEPGRAM_API_KEY", ""),
        ("DEEPGRAM_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn deepgram_transcription_settings_read_each_variable_alone() {
    let (key_only, base_beside_key) =
        Sections::deepgram_transcript_settings(&lookup(&[("DEEPGRAM_API_KEY", "k")]));
    let (key_beside_base, base_only) = Sections::deepgram_transcript_settings(&lookup(&[(
        "DEEPGRAM_API_BASE",
        "https://eu.test/v1",
    )]));

    assert_eq!(
        key_only.as_ref().map(ExposeSecret::expose_secret),
        Some("k")
    );
    assert_eq!(base_beside_key, DEFAULT_API_BASE);
    assert!(key_beside_base.is_none());
    assert_eq!(base_only, "https://eu.test/v1");
}
