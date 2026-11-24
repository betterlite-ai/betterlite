#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::sso_client_config;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn complete(flag: Option<&str>) -> BTreeMap<String, String> {
    let mut pairs = vec![
        ("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"),
        ("GENERIC_CLIENT_ID", "client-id"),
        ("GENERIC_CLIENT_SECRET", "client-secret"),
    ];
    if let Some(value) = flag {
        pairs.push(("GENERIC_INCLUDE_CLIENT_ID", value));
    }
    env(&pairs)
}

#[test]
fn reads_the_three_generic_variables() {
    let config = sso_client_config(&complete(None)).expect("the client is configured");
    assert_eq!(config.token_endpoint, "https://idp.example/token");
    assert_eq!(config.client_id, "client-id");
    assert_eq!(config.client_secret.expose_secret(), "client-secret");
    assert_eq!(
        config.auth_method,
        TokenEndpointAuthMethod::ClientSecretBasic
    );
}

#[rstest]
#[case::lowercase_true("true", TokenEndpointAuthMethod::ClientSecretPost)]
#[case::uppercase_true("TRUE", TokenEndpointAuthMethod::ClientSecretPost)]
#[case::mixed_case_true("tRuE", TokenEndpointAuthMethod::ClientSecretPost)]
#[case::false_flag("false", TokenEndpointAuthMethod::ClientSecretBasic)]
#[case::yes_is_not_true("yes", TokenEndpointAuthMethod::ClientSecretBasic)]
#[case::one_is_not_true("1", TokenEndpointAuthMethod::ClientSecretBasic)]
#[case::empty_flag("", TokenEndpointAuthMethod::ClientSecretBasic)]
fn the_include_client_id_flag_picks_the_auth_method(
    #[case] flag: &str,
    #[case] expected: TokenEndpointAuthMethod,
) {
    let config = sso_client_config(&complete(Some(flag))).expect("the client is configured");
    assert_eq!(config.auth_method, expected);
}

#[test]
fn an_absent_include_client_id_flag_uses_basic_auth() {
    let config = sso_client_config(&complete(None)).expect("the client is configured");
    assert_eq!(
        config.auth_method,
        TokenEndpointAuthMethod::ClientSecretBasic
    );
}

#[rstest]
#[case::no_token_endpoint(&[("GENERIC_CLIENT_ID", "client-id"), ("GENERIC_CLIENT_SECRET", "client-secret")])]
#[case::no_client_id(&[("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"), ("GENERIC_CLIENT_SECRET", "client-secret")])]
#[case::no_client_secret(&[("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"), ("GENERIC_CLIENT_ID", "client-id")])]
fn returns_none_when_a_variable_is_missing(#[case] pairs: &[(&str, &str)]) {
    assert!(sso_client_config(&env(pairs)).is_none());
}

#[rstest]
#[case::empty_token_endpoint(&[("GENERIC_TOKEN_ENDPOINT", ""), ("GENERIC_CLIENT_ID", "client-id"), ("GENERIC_CLIENT_SECRET", "client-secret")])]
#[case::empty_client_id(&[("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"), ("GENERIC_CLIENT_ID", ""), ("GENERIC_CLIENT_SECRET", "client-secret")])]
#[case::empty_client_secret(&[("GENERIC_TOKEN_ENDPOINT", "https://idp.example/token"), ("GENERIC_CLIENT_ID", "client-id"), ("GENERIC_CLIENT_SECRET", "")])]
fn returns_none_when_a_variable_is_empty(#[case] pairs: &[(&str, &str)]) {
    assert!(sso_client_config(&env(pairs)).is_none());
}

#[test]
fn the_debug_output_names_the_client_without_its_secret() {
    let config = sso_client_config(&complete(None)).expect("the client is configured");
    let rendered = format!("{config:?}");
    assert!(!rendered.contains("client-secret"));
    assert!(rendered.contains("token_endpoint"));
    assert!(rendered.contains("client-id"));
}
