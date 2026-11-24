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
#[case::passes_api_key("api_key", McpAuthType::ApiKey, Some("shared"), true)]
#[case::passes_bearer("bearer_token", McpAuthType::BearerToken, Some("abc"), true)]
#[case::passes_bearer_scheme("bearer_token", McpAuthType::BearerToken, Some("Bearer abc"), true)]
#[case::passes_token("token", McpAuthType::Token, Some("token abc"), true)]
#[case::passes_authorization("authorization", McpAuthType::Authorization, Some("Bearer abc"), true)]
#[case::passes_basic("basic", McpAuthType::Basic, Some("user:pass"), true)]
#[case::passes_basic_encoded("basic", McpAuthType::Basic, Some("Basic dXNlcjpwYXNz"), true)]
#[case::missing_api_key("api_key", McpAuthType::ApiKey, None, false)]
#[case::missing_bearer("bearer_token", McpAuthType::BearerToken, None, false)]
#[case::missing_basic("basic", McpAuthType::Basic, None, false)]
#[case::missing_token("token", McpAuthType::Token, None, false)]
#[case::missing_authorization("authorization", McpAuthType::Authorization, None, false)]
#[case::bare_bearer("bearer_token", McpAuthType::BearerToken, Some("Bearer"), false)]
#[case::bare_token("token", McpAuthType::Token, Some("token token"), false)]
#[case::bare_basic("basic", McpAuthType::Basic, Some("user"), false)]
fn prepare_handles_the_static_credential(
    #[case] auth_type: &str,
    #[case] client_auth_type: McpAuthType,
    #[case] value: Option<&str>,
    #[case] usable: bool,
) {
    let record = server_with(&format!(r#","auth_type":"{auth_type}""#));
    let auth_value = value.map(|text| AuthValue::Text(secret(text)));
    let mcp = client(client_auth_type, auth_value);
    let prepared = prepare_mcp_client(&record, &mcp);
    if usable {
        prepared.unwrap();
        return;
    }
    let expected =
        CredError::Misconfigured(format!("{auth_type} requires a usable upstream credential"));
    assert_eq!(prepared.unwrap_err(), expected);
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
