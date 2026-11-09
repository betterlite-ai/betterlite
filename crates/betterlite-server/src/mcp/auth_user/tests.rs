#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{McpAuthFields, McpAuthenticatedUser, REDACTED};

const SHARED_HEADER: &str = "shared-header-value";
const SERVER_HEADER: &str = "server-header-value";
const OAUTH2_HEADER: &str = "oauth2-header-value";
const RAW_HEADER: &str = "raw-header-value";
const CLIENT_IP: &str = "192.0.2.10";
const PROTOCOL_VERSION: &str = "2025-06-18";

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn principal() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).expect("fixture should construct")
}

fn authorization_header(value: &str) -> BTreeMap<String, SecretString> {
    BTreeMap::from([("Authorization".to_owned(), secret(value))])
}

fn server_auth_headers() -> BTreeMap<String, BTreeMap<String, SecretString>> {
    BTreeMap::from([("server-a".to_owned(), authorization_header(SERVER_HEADER))])
}

fn fields_with_server_headers(
    headers: BTreeMap<String, BTreeMap<String, SecretString>>,
) -> McpAuthFields {
    McpAuthFields {
        user_api_key_auth: None,
        mcp_auth_header: None,
        mcp_servers: None,
        mcp_server_auth_headers: Some(headers),
        oauth2_headers: None,
        mcp_protocol_version: None,
        raw_headers: None,
        client_ip: None,
    }
}

fn populated() -> McpAuthenticatedUser {
    McpAuthenticatedUser::new(McpAuthFields {
        user_api_key_auth: Some(principal()),
        mcp_auth_header: Some(secret(SHARED_HEADER)),
        mcp_servers: Some(vec!["server-a".to_owned()]),
        mcp_server_auth_headers: Some(server_auth_headers()),
        oauth2_headers: Some(authorization_header(OAUTH2_HEADER)),
        mcp_protocol_version: Some(PROTOCOL_VERSION.to_owned()),
        raw_headers: Some(authorization_header(RAW_HEADER)),
        client_ip: Some(CLIENT_IP.to_owned()),
    })
}

#[test]
fn defaults_every_optional_field_to_none_and_server_headers_to_an_empty_map() {
    let user = McpAuthenticatedUser::new(McpAuthFields::default());
    assert!(user.user_api_key_auth.is_none());
    assert!(user.mcp_auth_header.is_none());
    assert!(user.mcp_servers.is_none());
    assert!(user.mcp_server_auth_headers.is_empty());
    assert!(user.oauth2_headers.is_none());
    assert!(user.mcp_protocol_version.is_none());
    assert!(user.raw_headers.is_none());
    assert!(user.client_ip.is_none());
}

#[test]
fn keeps_every_field_it_was_given() {
    let user = populated();
    assert!(user.user_api_key_auth.is_some());
    assert_eq!(
        user.mcp_auth_header
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(SHARED_HEADER)
    );
    assert_eq!(user.mcp_servers, Some(vec!["server-a".to_owned()]));
    let server_headers = user
        .mcp_server_auth_headers
        .get("server-a")
        .expect("server-a headers");
    assert_eq!(
        server_headers
            .get("Authorization")
            .expect("authorization")
            .expose_secret(),
        SERVER_HEADER
    );
    assert_eq!(
        user.oauth2_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(OAUTH2_HEADER)
    );
    assert_eq!(user.mcp_protocol_version.as_deref(), Some(PROTOCOL_VERSION));
    assert_eq!(
        user.raw_headers
            .as_ref()
            .and_then(|headers| headers.get("Authorization"))
            .map(ExposeSecret::expose_secret),
        Some(RAW_HEADER)
    );
    assert_eq!(user.client_ip.as_deref(), Some(CLIENT_IP));
}

#[test]
fn an_explicit_empty_server_header_map_stays_empty() {
    let user = McpAuthenticatedUser::new(fields_with_server_headers(BTreeMap::new()));
    assert!(user.mcp_server_auth_headers.is_empty());
}

#[rstest]
#[case::mcp_auth_header(SHARED_HEADER)]
#[case::server_auth_header(SERVER_HEADER)]
#[case::oauth2_header(OAUTH2_HEADER)]
#[case::raw_header(RAW_HEADER)]
fn debug_never_prints_a_header_value_or_the_mcp_auth_header(#[case] hidden: &str) {
    let printed = format!("{:?}", populated());
    assert!(printed.contains("McpAuthenticatedUser"));
    assert!(printed.contains(REDACTED));
    assert!(!printed.contains(hidden));
}

#[test]
fn debug_prints_the_values_that_are_not_credentials() {
    let printed = format!("{:?}", populated());
    assert!(printed.contains("server-a"));
    assert!(printed.contains(PROTOCOL_VERSION));
    assert!(printed.contains(CLIENT_IP));
}
