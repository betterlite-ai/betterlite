#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://api.parallel.ai";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn parallel_ai_search_settings_default_to_an_unset_key() {
    let (api_key, api_base) = Sections::parallel_ai_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn parallel_ai_search_settings_read_the_environment() {
    let (api_key, api_base) = Sections::parallel_ai_search_settings(&lookup(&[
        ("PARALLEL_AI_API_KEY", "ai-key"),
        ("PARALLEL_AI_API_BASE", "https://proxy.test"),
    ]));

    assert_eq!(
        api_key.as_ref().map(|key| key.expose_secret().to_owned()),
        Some("ai-key".to_owned())
    );
    assert_eq!(api_base, "https://proxy.test");
}

#[test]
fn parallel_ai_search_settings_read_the_legacy_key() {
    let (api_key, api_base) =
        Sections::parallel_ai_search_settings(&lookup(&[("PARALLEL_API_KEY", "legacy-key")]));

    assert_eq!(
        api_key.as_ref().map(|key| key.expose_secret().to_owned()),
        Some("legacy-key".to_owned())
    );
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn parallel_ai_search_settings_prefer_the_ai_key() {
    let (api_key, _api_base) = Sections::parallel_ai_search_settings(&lookup(&[
        ("PARALLEL_AI_API_KEY", "ai-key"),
        ("PARALLEL_API_KEY", "legacy-key"),
    ]));

    assert_eq!(
        api_key.as_ref().map(|key| key.expose_secret().to_owned()),
        Some("ai-key".to_owned())
    );
}

#[test]
fn parallel_ai_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, api_base) = Sections::parallel_ai_search_settings(&lookup(&[
        ("PARALLEL_AI_API_KEY", ""),
        ("PARALLEL_AI_API_BASE", ""),
        ("PARALLEL_API_KEY", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn parallel_ai_search_settings_fall_through_one_empty_key() {
    let (api_key, _api_base) = Sections::parallel_ai_search_settings(&lookup(&[
        ("PARALLEL_AI_API_KEY", ""),
        ("PARALLEL_API_KEY", "legacy-key"),
    ]));

    assert_eq!(
        api_key.as_ref().map(|key| key.expose_secret().to_owned()),
        Some("legacy-key".to_owned())
    );
}
