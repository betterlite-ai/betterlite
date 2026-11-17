#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

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

#[rstest]
#[case::nothing_set(&[], SsoMappingSources::default())]
#[case::client_secret(
    &[("GENERIC_CLIENT_SECRET", "secret")],
    SsoMappingSources { client_secret: Some(SecretString::from("secret")), ..Default::default() }
)]
#[case::scope(
    &[("GENERIC_SCOPE", "openid email")],
    SsoMappingSources { scope: Some("openid email".to_owned()), ..Default::default() }
)]
#[case::authorization_endpoint(
    &[("GENERIC_AUTHORIZATION_ENDPOINT", "https://idp.example/authorize")],
    SsoMappingSources {
        authorization_endpoint: Some("https://idp.example/authorize".to_owned()),
        ..Default::default()
    }
)]
#[case::token_endpoint(
    &[("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token")],
    SsoMappingSources { token_endpoint: Some("https://idp.example/token".to_owned()), ..Default::default() }
)]
#[case::userinfo_endpoint(
    &[("GENERIC_USERINFO_ENDPOINT", "https://idp.example/userinfo")],
    SsoMappingSources { userinfo_endpoint: Some("https://idp.example/userinfo".to_owned()), ..Default::default() }
)]
#[case::include_client_id(
    &[("GENERIC_INCLUDE_CLIENT_ID", "TRUE")],
    SsoMappingSources { include_client_id: true, ..Default::default() }
)]
#[case::role_mappings_roles(
    &[("GENERIC_ROLE_MAPPINGS_ROLES", "{'proxy_admin': ['admins']}")],
    SsoMappingSources { role_mappings_roles: Some("{'proxy_admin': ['admins']}".to_owned()), ..Default::default() }
)]
#[case::role_mappings_group_claim(
    &[("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "groups")],
    SsoMappingSources { role_mappings_group_claim: Some("groups".to_owned()), ..Default::default() }
)]
#[case::role_mappings_default_role(
    &[("GENERIC_ROLE_MAPPINGS_DEFAULT_ROLE", "internal_user")],
    SsoMappingSources { role_mappings_default_role: Some("internal_user".to_owned()), ..Default::default() }
)]
#[case::sso_headers(
    &[("GENERIC_SSO_HEADERS", "X-Team=eng")],
    SsoMappingSources { sso_headers: Some("X-Team=eng".to_owned()), ..Default::default() }
)]
fn loads_mapping_sources(#[case] pairs: &[(&str, &str)], #[case] expected: SsoMappingSources) {
    let actual = load_mapping_sources(&environment(pairs));
    assert_eq!(secret(&actual), secret(&expected));
    assert_eq!(actual.scope, expected.scope);
    assert_eq!(
        actual.authorization_endpoint,
        expected.authorization_endpoint
    );
    assert_eq!(actual.token_endpoint, expected.token_endpoint);
    assert_eq!(actual.userinfo_endpoint, expected.userinfo_endpoint);
    assert_eq!(actual.include_client_id, expected.include_client_id);
    assert_eq!(actual.role_mappings_roles, expected.role_mappings_roles);
    assert_eq!(
        actual.role_mappings_group_claim,
        expected.role_mappings_group_claim
    );
    assert_eq!(
        actual.role_mappings_default_role,
        expected.role_mappings_default_role
    );
    assert_eq!(actual.sso_headers, expected.sso_headers);
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
