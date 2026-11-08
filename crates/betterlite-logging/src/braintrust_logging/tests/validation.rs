use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::braintrust_logging::settings::{Settings, SettingsError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn validation_rejects_a_missing_api_key() {
    let error = Settings::resolve(None, None, Vec::new(), &environment(&[])).unwrap_err();
    assert_eq!(error, SettingsError::MissingApiKey);
    assert!(error.to_string().contains("BRAINTRUST_API_KEY"));
}

#[test]
fn validation_accepts_an_argument_key() {
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        None,
        Vec::new(),
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "arg-key");
}

#[test]
fn validation_rejects_an_empty_argument_key() {
    let error = Settings::resolve(
        Some(SecretString::from("")),
        None,
        Vec::new(),
        &environment(&[]),
    )
    .unwrap_err();
    assert_eq!(error, SettingsError::MissingApiKey);
}

#[test]
fn validation_reads_the_key_from_the_environment() {
    let resolved = Settings::resolve(
        None,
        None,
        Vec::new(),
        &environment(&[("BRAINTRUST_API_KEY", "env-key")]),
    )
    .unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "env-key");
}
