#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret;

use super::{SsoMappingSources, load_mapping_sources};

fn secret(sources: &SsoMappingSources) -> Option<&str> {
    sources
        .client_secret
        .as_ref()
        .map(ExposeSecret::expose_secret)
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn loads_mapping_sources() {
    let absent = load_mapping_sources(&BTreeMap::<String, String>::new());
    assert!(absent.client_secret.is_none());
    assert!(absent.scope.is_none());
    assert!(absent.authorization_endpoint.is_none());
    assert!(!absent.include_client_id);
    assert!(absent.sso_headers.is_none());
    let configured = load_mapping_sources(&environment(&[
        ("GENERIC_CLIENT_SECRET", "secret"),
        ("GENERIC_SCOPE", "openid email"),
        (
            "GENERIC_AUTHORIZATION_ENDPOINT",
            "https://idp.example/authorize",
        ),
        ("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"),
        ("GENERIC_USERINFO_ENDPOINT", "https://idp.example/userinfo"),
        ("GENERIC_INCLUDE_CLIENT_ID", "TRUE"),
        ("GENERIC_ROLE_MAPPINGS_ROLES", "{'proxy_admin': ['admins']}"),
        ("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "groups"),
        ("GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE", "internal_user"),
        ("GENERIC_SSO_HEADERS", "X-Team=eng"),
    ]));
    assert_eq!(secret(&configured), Some("secret"));
    assert_eq!(configured.scope.as_deref(), Some("openid email"));
    assert_eq!(
        configured.authorization_endpoint.as_deref(),
        Some("https://idp.example/authorize")
    );
    assert_eq!(
        configured.token_endpoint.as_deref(),
        Some("https://idp.example/token")
    );
    assert_eq!(
        configured.userinfo_endpoint.as_deref(),
        Some("https://idp.example/userinfo")
    );
    assert!(configured.include_client_id);
    assert_eq!(
        configured.role_mappings_roles.as_deref(),
        Some("{'proxy_admin': ['admins']}")
    );
    assert_eq!(
        configured.role_mappings_group_claim.as_deref(),
        Some("groups")
    );
    assert_eq!(
        configured.role_mappings_default_role.as_deref(),
        Some("internal_user")
    );
    assert_eq!(configured.sso_headers.as_deref(), Some("X-Team=eng"));
}

#[test]
fn include_client_id_is_true_in_any_case_and_only_for_the_bare_word() {
    let flags = ["true", "TRUE", "True", "tRuE"];
    assert!(flags.into_iter().all(|value| {
        load_mapping_sources(&environment(&[("GENERIC_INCLUDE_CLIENT_ID", value)]))
            .include_client_id
    }));
    let off = [" true", "true ", "1", "yes", "on", "", "false"];
    assert!(off.into_iter().all(|value| {
        !load_mapping_sources(&environment(&[("GENERIC_INCLUDE_CLIENT_ID", value)]))
            .include_client_id
    }));
    let absent = load_mapping_sources(&environment(&[("GENERIC_SCOPE", "openid")]));
    assert!(!absent.include_client_id);
}

#[test]
fn an_empty_source_value_stays_set() {
    let sources = load_mapping_sources(&environment(&[
        ("GENERIC_CLIENT_SECRET", ""),
        ("GENERIC_SCOPE", ""),
        ("GENERIC_SSO_HEADERS", ""),
        ("GENERIC_ROLE_MAPPINGS_ROLES", ""),
    ]));
    assert_eq!(secret(&sources), Some(""));
    assert_eq!(sources.scope.as_deref(), Some(""));
    assert_eq!(sources.sso_headers.as_deref(), Some(""));
    assert_eq!(sources.role_mappings_roles.as_deref(), Some(""));
    assert!(sources.token_endpoint.is_none());
    assert!(sources.role_mappings_group_claim.is_none());
}

#[test]
fn each_source_reads_its_own_environment_variable() {
    let sources = load_mapping_sources(&environment(&[(
        "GENERIC_USERINFO_ENDPOINT",
        "https://idp.example/userinfo",
    )]));
    assert_eq!(
        sources.userinfo_endpoint.as_deref(),
        Some("https://idp.example/userinfo")
    );
    assert!(sources.authorization_endpoint.is_none());
    assert!(sources.token_endpoint.is_none());
    assert!(sources.client_secret.is_none());
    assert!(sources.scope.is_none());
}
