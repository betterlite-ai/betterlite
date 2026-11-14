#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://sdk.nimbleway.com/v2";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn nimble_search_settings_default_to_an_unset_key() {
    let (api_key, api_base) = Sections::nimble_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn nimble_search_settings_read_the_environment() {
    let (api_key, api_base) = Sections::nimble_search_settings(&lookup(&[
        ("NIMBLE_API_KEY", "server-key"),
        ("NIMBLE_API_BASE", "https://proxy.test/v2"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(api_base, "https://proxy.test/v2");
}

#[test]
fn nimble_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, api_base) = Sections::nimble_search_settings(&lookup(&[
        ("NIMBLE_API_KEY", ""),
        ("NIMBLE_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}
