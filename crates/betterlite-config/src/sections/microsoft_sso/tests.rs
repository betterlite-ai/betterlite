#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use super::{
    AUTHORIZATION_ENDPOINT_ENV, CLIENT_SECRET_ENV, Settings, TENANT_ENV, TOKEN_ENDPOINT_ENV,
    USERINFO_ENDPOINT_ENV,
};
use crate::loader::ConfigLoader;
use crate::types::Config;

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config: Config = ConfigLoader { lookup }.load("").unwrap();
    config.sections.microsoft_sso
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[test]
fn loads_microsoft_sso_settings_with_every_variable_absent() {
    let settings = load(&[]);
    assert!(settings.client_secret.is_none());
    assert!(settings.tenant.is_none());
    assert!(settings.authorization_endpoint.is_none());
    assert!(settings.token_endpoint.is_none());
    assert!(settings.userinfo_endpoint.is_none());
}

#[test]
fn loads_microsoft_sso_settings_with_every_variable_set() {
    assert_eq!(CLIENT_SECRET_ENV, "MICROSOFT_CLIENT_SECRET");
    assert_eq!(TENANT_ENV, "MICROSOFT_TENANT");
    assert_eq!(
        AUTHORIZATION_ENDPOINT_ENV,
        "MICROSOFT_AUTHORIZATION_ENDPOINT"
    );
    assert_eq!(TOKEN_ENDPOINT_ENV, "MICROSOFT_TOKEN_ENDPOINT");
    assert_eq!(USERINFO_ENDPOINT_ENV, "MICROSOFT_USERINFO_ENDPOINT");
    let settings = load(&[
        (CLIENT_SECRET_ENV, "microsoft-secret"),
        (TENANT_ENV, "contoso.example.test"),
        (
            AUTHORIZATION_ENDPOINT_ENV,
            "https://login.example.test/authorize",
        ),
        (TOKEN_ENDPOINT_ENV, "https://login.example.test/token"),
        (USERINFO_ENDPOINT_ENV, "https://graph.example.test/me"),
    ]);
    assert_eq!(
        revealed(settings.client_secret.as_ref()),
        Some("microsoft-secret")
    );
    assert_eq!(settings.tenant.as_deref(), Some("contoso.example.test"));
    assert_eq!(
        settings.authorization_endpoint.as_deref(),
        Some("https://login.example.test/authorize")
    );
    assert_eq!(
        settings.token_endpoint.as_deref(),
        Some("https://login.example.test/token")
    );
    assert_eq!(
        settings.userinfo_endpoint.as_deref(),
        Some("https://graph.example.test/me")
    );
    assert!(!format!("{settings:?}").contains("microsoft-secret"));
}

#[test]
fn loads_microsoft_sso_settings_with_every_variable_empty() {
    let settings = load(&[
        (CLIENT_SECRET_ENV, ""),
        (TENANT_ENV, ""),
        (AUTHORIZATION_ENDPOINT_ENV, ""),
        (TOKEN_ENDPOINT_ENV, ""),
        (USERINFO_ENDPOINT_ENV, ""),
    ]);
    assert_eq!(revealed(settings.client_secret.as_ref()), Some(""));
    assert_eq!(settings.tenant.as_deref(), Some(""));
    assert_eq!(settings.authorization_endpoint.as_deref(), Some(""));
    assert_eq!(settings.token_endpoint.as_deref(), Some(""));
    assert_eq!(settings.userinfo_endpoint.as_deref(), Some(""));
}
