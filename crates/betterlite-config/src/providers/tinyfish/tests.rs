#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::types::LiteLlmSettings;

const DEFAULT_API_BASE: &str = "https://api.search.tinyfish.ai";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn tinyfish_search_settings_default_to_an_unset_key() {
    let (api_key, api_base) = LiteLlmSettings::tinyfish_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn tinyfish_search_settings_read_the_environment() {
    let (api_key, api_base) = LiteLlmSettings::tinyfish_search_settings(&lookup(&[
        ("TINYFISH_API_KEY", "server-key"),
        ("TINYFISH_API_BASE", "https://proxy.test/search"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test/search");
}

#[test]
fn tinyfish_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, api_base) = LiteLlmSettings::tinyfish_search_settings(&lookup(&[
        ("TINYFISH_API_KEY", ""),
        ("TINYFISH_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}
