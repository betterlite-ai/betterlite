#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::providers::searchapi::SearchApiSearchSettings;

const DEFAULT_API_BASE: &str = "https://www.searchapi.io/api/v1/search";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn searchapi_search_settings_default_to_an_unset_key() {
    let settings = SearchApiSearchSettings::load(&lookup(&[]));

    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
}

#[test]
fn searchapi_search_settings_read_the_environment() {
    let settings = SearchApiSearchSettings::load(&lookup(&[
        ("SEARCHAPI_API_KEY", "server-key"),
        ("SEARCHAPI_API_BASE", "https://proxy.test/v1"),
    ]));

    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(settings.api_base, "https://proxy.test/v1");
}

#[test]
fn searchapi_search_settings_treat_an_empty_variable_as_unset() {
    let settings = SearchApiSearchSettings::load(&lookup(&[
        ("SEARCHAPI_API_KEY", ""),
        ("SEARCHAPI_API_BASE", ""),
    ]));

    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
}
