#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::mistral_options::Settings;

const CONFIGURED: [(&str, &str); 4] = [
    ("DEFAULT_MAX_RECURSE_DEPTH", "4"),
    ("MISTRAL_AZURE_API_BASE", "https://azure.example"),
    ("MISTRAL_AZURE_API_KEY", "azure-key"),
    ("MISTRAL_API_KEY", "mistral-key"),
];

const EMPTY: [(&str, &str); 4] = [
    ("DEFAULT_MAX_RECURSE_DEPTH", ""),
    ("MISTRAL_AZURE_API_BASE", ""),
    ("MISTRAL_AZURE_API_KEY", ""),
    ("MISTRAL_API_KEY", ""),
];

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load("")?.sections.mistral_options)
}

fn assert_unset(settings: &Settings) {
    assert!(settings.azure_api_base.is_none());
    assert!(settings.azure_api_key.is_none());
    assert!(settings.api_key.is_none());
}

#[test]
fn mistral_options_settings() {
    let defaults = load(&[]).expect("the defaults load");
    assert_eq!(defaults.max_recurse_depth, 100);
    assert_unset(&defaults);

    let configured = load(&CONFIGURED).expect("the configured values load");
    assert_eq!(configured.max_recurse_depth, 4);
    assert_eq!(
        configured.azure_api_base.as_deref(),
        Some("https://azure.example")
    );
    assert_eq!(
        configured
            .azure_api_key
            .as_ref()
            .expect("an azure key")
            .expose_secret(),
        "azure-key"
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
    assert_eq!(empty.max_recurse_depth, 100);
    assert_unset(&empty);
}

#[test]
fn mistral_options_settings_reject_a_non_integer_depth() {
    let invalid = load(&[("DEFAULT_MAX_RECURSE_DEPTH", "many")])
        .expect_err("a non-integer depth fails the load");
    assert!(matches!(invalid, ConfigError::Environment { .. }));
    assert!(invalid.to_string().contains("DEFAULT_MAX_RECURSE_DEPTH"));
}
