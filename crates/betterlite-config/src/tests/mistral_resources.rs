#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::mistral_resources::Settings;

const CONFIGURED: [(&str, &str); 2] = [
    ("MISTRAL_API_BASE", "https://mistral.example"),
    ("MISTRAL_API_KEY", "mistral-key"),
];

const EMPTY: [(&str, &str); 4] = [
    ("MISTRAL_API_BASE", ""),
    ("MISTRAL_API_KEY", ""),
    ("MISTRAL_AZURE_API_BASE", "https://azure.example"),
    ("MISTRAL_AZURE_API_KEY", "azure-key"),
];

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.mistral_resources)
}

#[test]
fn mistral_resource_settings() {
    let defaults = load(&[]).expect("the defaults load");
    assert!(defaults.api_base.is_none());
    assert!(defaults.api_key.is_none());

    let configured = load(&CONFIGURED).expect("the configured values load");
    assert_eq!(
        configured.api_base.as_deref(),
        Some("https://mistral.example")
    );
    assert_eq!(
        configured
            .api_key
            .as_ref()
            .expect("a mistral key")
            .expose_secret(),
        "mistral-key"
    );

    let empty = load(&EMPTY).expect("an empty value counts as unset");
    assert!(empty.api_base.is_none());
    assert!(empty.api_key.is_none());
}

#[test]
fn mistral_resource_settings_ignore_the_azure_keys() {
    let settings =
        load(&[("MISTRAL_AZURE_API_BASE", "https://azure.example")]).expect("the azure base loads");
    assert!(settings.api_base.is_none());
    let key_only = load(&[("MISTRAL_AZURE_API_KEY", "azure-key")]).expect("the azure key loads");
    assert!(key_only.api_key.is_none());
}
