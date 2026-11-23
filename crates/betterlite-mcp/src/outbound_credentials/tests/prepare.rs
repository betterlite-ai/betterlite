use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::credential_contracts::errors::CredError;
use crate::credentials::{AuthValue, McpAuthType, McpClient};

use super::super::adapter::prepare::prepare_mcp_client;
use super::support::{FailingAuth, client, client_with, secret, server_from};

fn server_with(extra: &str) -> crate::server_record::MCPServer {
    let encoded = format!(
        r#"{{"server_id":"srv-1","name":"Server One","transport":"http","url":"https://mcp.example.com/mcp"{extra}}}"#
    );
    server_from(&encoded)
}

fn stdio_server(extra: &str) -> crate::server_record::MCPServer {
    let encoded =
        format!(r#"{{"server_id":"srv-1","name":"Server One","transport":"stdio"{extra}}}"#);
    server_from(&encoded)
}

#[rstest]
#[case::no_auth_type("")]
#[case::none(r#","auth_type":"none""#)]
#[case::oauth2(r#","auth_type":"oauth2""#)]
#[case::oauth2_token_exchange(r#","auth_type":"oauth2_token_exchange""#)]
#[case::oauth2_id_jag(r#","auth_type":"oauth2_id_jag""#)]
#[case::aws_sigv4(r#","auth_type":"aws_sigv4""#)]
#[case::true_passthrough(r#","auth_type":"true_passthrough""#)]
#[case::oauth_delegate(r#","auth_type":"oauth_delegate""#)]
fn prepare_skips_non_static_mode(#[case] extra: &str) {
    let record = server_with(extra);
    let mcp = client(McpAuthType::None, None);
    prepare_mcp_client(&record, &mcp).unwrap();
}

#[rstest]
#[case::api_key(r#","auth_type":"api_key""#)]
#[case::bearer_token(r#","auth_type":"bearer_token""#)]
#[case::basic(r#","auth_type":"basic""#)]
fn prepare_skips_stdio_transport(#[case] extra: &str) {
    let record = stdio_server(extra);
    let mcp = client(McpAuthType::ApiKey, None);
    prepare_mcp_client(&record, &mcp).unwrap();
}

#[rstest]
#[case::api_key("api_key", McpAuthType::ApiKey, "shared")]
#[case::bearer_token("bearer_token", McpAuthType::BearerToken, "abc")]
#[case::bearer_token_with_scheme("bearer_token", McpAuthType::BearerToken, "Bearer abc")]
#[case::token("token", McpAuthType::Token, "token abc")]
#[case::authorization("authorization", McpAuthType::Authorization, "Bearer abc")]
#[case::basic("basic", McpAuthType::Basic, "user:pass")]
#[case::basic_encoded("basic", McpAuthType::Basic, "Basic dXNlcjpwYXNz")]
fn prepare_passes_usable_static_header(
    #[case] auth_type: &str,
    #[case] client_auth_type: McpAuthType,
    #[case] value: &str,
) {
    let record = server_with(&format!(r#","auth_type":"{auth_type}""#));
    let mcp = client(client_auth_type, Some(AuthValue::Text(secret(value))));
    prepare_mcp_client(&record, &mcp).unwrap();
}

#[test]
fn prepare_counts_an_authorization_extra_header() {
    let record = server_with(r#","auth_type":"api_key""#);
    let extra = BTreeMap::from([("Authorization".to_owned(), "Bearer abc".to_owned())]);
    let mcp = client_with(McpAuthType::ApiKey, None, extra);
    prepare_mcp_client(&record, &mcp).unwrap();
}

#[test]
fn prepare_counts_an_admin_static_header() {
    let record = server_with(r#","auth_type":"api_key","static_headers":{"apikey":"static-key"}"#);
    let extra = BTreeMap::from([("apikey".to_owned(), "static-key".to_owned())]);
    let mcp = client_with(McpAuthType::ApiKey, None, extra);
    prepare_mcp_client(&record, &mcp).unwrap();
}

#[rstest]
#[case::api_key(
    "api_key",
    McpAuthType::ApiKey,
    "api_key requires a usable upstream credential"
)]
#[case::bearer_token(
    "bearer_token",
    McpAuthType::BearerToken,
    "bearer_token requires a usable upstream credential"
)]
#[case::basic(
    "basic",
    McpAuthType::Basic,
    "basic requires a usable upstream credential"
)]
#[case::token(
    "token",
    McpAuthType::Token,
    "token requires a usable upstream credential"
)]
#[case::authorization(
    "authorization",
    McpAuthType::Authorization,
    "authorization requires a usable upstream credential"
)]
fn prepare_raises_misconfigured_without_credential(
    #[case] auth_type: &str,
    #[case] client_auth_type: McpAuthType,
    #[case] message: &str,
) {
    let record = server_with(&format!(r#","auth_type":"{auth_type}""#));
    let mcp = client(client_auth_type, None);
    let error = prepare_mcp_client(&record, &mcp).unwrap_err();
    assert_eq!(error, CredError::Misconfigured(message.to_owned()));
}

#[rstest]
#[case::bearer(McpAuthType::BearerToken, "bearer_token", "Bearer")]
#[case::token(McpAuthType::Token, "token", "token token")]
#[case::basic_without_colon(McpAuthType::Basic, "basic", "user")]
fn prepare_refuses_a_bare_scheme(
    #[case] client_auth_type: McpAuthType,
    #[case] auth_type: &str,
    #[case] value: &str,
) {
    let record = server_with(&format!(r#","auth_type":"{auth_type}""#));
    let mcp = client(client_auth_type, Some(AuthValue::Text(secret(value))));
    let error = prepare_mcp_client(&record, &mcp).unwrap_err();
    assert_eq!(
        error,
        CredError::Misconfigured(format!("{auth_type} requires a usable upstream credential"))
    );
}

#[test]
fn prepare_maps_a_prepare_failure() {
    let record = server_with(r#","auth_type":"api_key""#);
    let mcp = McpClient::new(
        "https://mcp.example.com/mcp".to_owned(),
        McpAuthType::ApiKey,
        None,
        None,
        BTreeMap::new(),
        Duration::from_secs(30),
        Some(Box::new(FailingAuth)),
        None,
    );
    let error = prepare_mcp_client(&record, &mcp).unwrap_err();
    let expected = CredError::Misconfigured(
        "could not prepare the upstream request: resolved authentication failed".to_owned(),
    );
    assert_eq!(error, expected);
}
