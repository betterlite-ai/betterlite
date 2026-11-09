#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use crate::sections::Sections;

const DEFAULT_API_BASE: &str = "https://www.googleapis.com/customsearch/v1";

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn google_pse_search_settings_default_to_unset_key_and_engine() {
    let (api_key, engine_id, api_base) = Sections::google_pse_search_settings(&lookup(&[]));

    assert!(api_key.is_none());
    assert!(engine_id.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}

#[test]
fn google_pse_search_settings_read_the_environment() {
    let (api_key, engine_id, api_base) = Sections::google_pse_search_settings(&lookup(&[
        ("GOOGLE_PSE_API_KEY", "server-key"),
        ("GOOGLE_PSE_ENGINE_ID", "engine-id"),
        ("GOOGLE_PSE_API_BASE", "https://proxy.test/v1"),
    ]));

    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("server-key")
    );
    assert_eq!(engine_id.as_deref(), Some("engine-id"));
    assert_eq!(api_base, "https://proxy.test/v1");
}

#[test]
fn google_pse_search_settings_treat_an_empty_variable_as_unset() {
    let (api_key, engine_id, api_base) = Sections::google_pse_search_settings(&lookup(&[
        ("GOOGLE_PSE_API_KEY", ""),
        ("GOOGLE_PSE_ENGINE_ID", ""),
        ("GOOGLE_PSE_API_BASE", ""),
    ]));

    assert!(api_key.is_none());
    assert!(engine_id.is_none());
    assert_eq!(api_base, DEFAULT_API_BASE);
}
