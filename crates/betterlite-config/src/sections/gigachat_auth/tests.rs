#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use crate::loader::ConfigLoader;
use crate::sections::gigachat_auth::{DEFAULT_AUTH_URL, DEFAULT_SCOPE, Settings};

fn settings_from(environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("a document with no gigachat_auth keys still loads")
        .sections
        .gigachat_auth
}

fn secret(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[test]
fn env_loads_all_four_variables() {
    let settings = settings_from(&[
        ("GIGACHAT_ACCESS_TOKEN", "direct-token"),
        ("GIGACHAT_CREDENTIALS", "credential-pair"),
        ("GIGACHAT_AUTH_URL", "https://oauth.example/v2/oauth"),
        ("GIGACHAT_SCOPE", "GIGACHAT_API_CORP"),
    ]);

    assert_eq!(secret(settings.access_token.as_ref()), Some("direct-token"));
    assert_eq!(
        secret(settings.credentials.as_ref()),
        Some("credential-pair")
    );
    assert_eq!(settings.auth_url, "https://oauth.example/v2/oauth");
    assert_eq!(settings.scope, "GIGACHAT_API_CORP");
}

#[test]
fn credentials_prefer_the_credentials_variable() {
    let settings = settings_from(&[
        ("GIGACHAT_CREDENTIALS", "credential-pair"),
        ("GIGACHAT_API_KEY", "api-key"),
    ]);

    assert_eq!(
        secret(settings.credentials.as_ref()),
        Some("credential-pair")
    );
}

#[test]
fn credentials_fall_back_to_the_api_key() {
    let settings = settings_from(&[("GIGACHAT_API_KEY", "api-key")]);

    assert_eq!(secret(settings.credentials.as_ref()), Some("api-key"));
}

#[test]
fn empty_credentials_fall_back_to_the_api_key() {
    let settings = settings_from(&[
        ("GIGACHAT_CREDENTIALS", ""),
        ("GIGACHAT_API_KEY", "api-key"),
    ]);

    assert_eq!(secret(settings.credentials.as_ref()), Some("api-key"));
}

#[test]
fn unset_values_take_the_defaults() {
    let settings = settings_from(&[]);

    assert!(settings.access_token.is_none());
    assert!(settings.credentials.is_none());
    assert_eq!(settings.auth_url, DEFAULT_AUTH_URL);
    assert_eq!(settings.scope, DEFAULT_SCOPE);
}

#[test]
fn empty_values_count_as_unset() {
    let settings = settings_from(&[
        ("GIGACHAT_ACCESS_TOKEN", ""),
        ("GIGACHAT_CREDENTIALS", ""),
        ("GIGACHAT_API_KEY", "api-key"),
        ("GIGACHAT_AUTH_URL", ""),
        ("GIGACHAT_SCOPE", ""),
    ]);

    assert!(settings.access_token.is_none());
    assert_eq!(secret(settings.credentials.as_ref()), Some("api-key"));
    assert_eq!(settings.auth_url, DEFAULT_AUTH_URL);
    assert_eq!(settings.scope, DEFAULT_SCOPE);
}

#[test]
fn debug_output_hides_the_secrets() {
    let settings = settings_from(&[
        ("GIGACHAT_ACCESS_TOKEN", "direct-token-value"),
        ("GIGACHAT_CREDENTIALS", "credential-pair-value"),
    ]);

    let printed = format!("{settings:?}");

    assert!(!printed.contains("direct-token-value"));
    assert!(!printed.contains("credential-pair-value"));
}
