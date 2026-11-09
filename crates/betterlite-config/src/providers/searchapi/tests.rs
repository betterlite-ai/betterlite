#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::types::Config;

const DEFAULT_API_BASE: &str = "https://www.searchapi.io/api/v1/search";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn searchapi_search_settings_default_to_an_unset_key() {
    let (api_key, api_base) = Config::searchapi_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn searchapi_search_settings_read_the_environment() {
    let (api_key, api_base) = Config::searchapi_search_settings(&lookup(&[
        ("SEARCHAPI_API_KEY", "server-key"),
        ("SEARCHAPI_API_BASE", "https://proxy.test/v1"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test/v1");
}

#[test]
fn searchapi_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, api_base) = Config::searchapi_search_settings(&lookup(&[
        ("SEARCHAPI_API_KEY", ""),
        ("SEARCHAPI_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}
