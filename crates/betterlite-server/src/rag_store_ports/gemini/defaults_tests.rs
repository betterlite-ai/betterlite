#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::defaults;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn google_key_comes_before_the_gemini_key() {
    let settings = defaults(&environment(&[
        ("GOOGLE_API_KEY", "google-key"),
        ("GEMINI_API_KEY", "gemini-key"),
    ]))
    .expect("the settings convert");
    let key = settings.api_key.expect("a key resolves");
    assert_eq!(key.expose_secret(), "google-key");
}

#[test]
fn the_gemini_key_is_used_without_a_google_key() {
    let settings =
        defaults(&environment(&[("GEMINI_API_KEY", "gemini-key")])).expect("the settings convert");
    let key = settings.api_key.expect("a key resolves");
    assert_eq!(key.expose_secret(), "gemini-key");
}

#[test]
fn an_empty_key_counts_as_unset() {
    let settings = defaults(&environment(&[
        ("GOOGLE_API_KEY", ""),
        ("GEMINI_API_KEY", "gemini-key"),
    ]))
    .expect("the settings convert");
    let key = settings.api_key.expect("the gemini key resolves");
    assert_eq!(key.expose_secret(), "gemini-key");
    let unset = defaults(&environment(&[
        ("GOOGLE_API_KEY", ""),
        ("GEMINI_API_KEY", ""),
    ]))
    .expect("the settings convert");
    assert!(unset.api_key.is_none());
}

#[test]
fn the_api_base_comes_from_the_environment() {
    let settings = defaults(&environment(&[(
        "GEMINI_API_BASE",
        "http://localhost:8080",
    )]))
    .expect("the settings convert");
    assert_eq!(settings.api_base, "http://localhost:8080");
}

#[test]
fn the_api_base_defaults_to_the_studio_base() {
    let settings = defaults(&environment(&[])).expect("the settings convert");
    assert_eq!(
        settings.api_base,
        "https://generativelanguage.googleapis.com"
    );
}

#[test]
fn an_empty_api_base_counts_as_unset() {
    let settings =
        defaults(&environment(&[("GEMINI_API_BASE", "")])).expect("the settings convert");
    assert_eq!(
        settings.api_base,
        "https://generativelanguage.googleapis.com"
    );
}

#[test]
fn the_key_stays_out_of_debug() {
    let settings =
        defaults(&environment(&[("GOOGLE_API_KEY", "google-key")])).expect("the settings convert");
    let printed = format!("{settings:?}");
    assert!(!printed.contains("google-key"));
    assert!(printed.contains("REDACTED"));
}
