#![cfg(test)]

use std::collections::BTreeMap;

use super::{SsoMappingSources, load_mapping_sources};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[test]
fn loads_mapping_sources() {
    let absent = load_mapping_sources(&BTreeMap::<String, String>::new());
    assert_eq!(absent, SsoMappingSources::default());
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
    assert_eq!(
        configured,
        SsoMappingSources {
            client_secret: Some("secret".to_owned()),
            scope: Some("openid email".to_owned()),
            authorization_endpoint: Some("https://idp.example/authorize".to_owned()),
            token_endpoint: Some("https://idp.example/token".to_owned()),
            userinfo_endpoint: Some("https://idp.example/userinfo".to_owned()),
            include_client_id: true,
            role_mappings_roles: Some("{'proxy_admin': ['admins']}".to_owned()),
            role_mappings_group_claim: Some("groups".to_owned()),
            role_mappings_default_role: Some("internal_user".to_owned()),
            sso_headers: Some("X-Team=eng".to_owned()),
        }
    );
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
    assert_eq!(sources.client_secret.as_deref(), Some(""));
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
