#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://apiserpent.com";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn apiserpent_search_settings_default_to_an_unset_key_and_the_source_base() {
    let (api_key, api_base) = Sections::apiserpent_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn apiserpent_search_settings_read_the_environment() {
    let (api_key, api_base) = Sections::apiserpent_search_settings(&lookup(&[
        ("APISERPENT_API_KEY", "server-key"),
        ("APISERPENT_API_BASE", "https://proxy.test"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test");
}

#[test]
fn apiserpent_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, api_base) = Sections::apiserpent_search_settings(&lookup(&[
        ("APISERPENT_API_KEY", ""),
        ("APISERPENT_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}
