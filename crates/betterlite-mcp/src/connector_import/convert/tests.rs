#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{
    ConnectorConversionError, ConnectorOutcome, ConvertedConnector, convert_connector_entries,
    sanitize_connector_name,
};
use crate::connector_import::request::ConnectorImportRequest;
use crate::server_payload::request::{NewMCPServerRequest, OrderedMap};

fn policy(enabled: bool) -> StdioPolicy {
    StdioPolicy {
        enabled,
        allowed_commands: ["node".to_owned(), "npx".to_owned()]
            .into_iter()
            .collect::<BTreeSet<String>>(),
    }
}

fn outcomes(source: &str, enabled: bool) -> Vec<ConnectorOutcome> {
    let request = ConnectorImportRequest::from_slice(source.as_bytes()).unwrap();
    convert_connector_entries(&request, &policy(enabled))
}

fn converted(source: &str) -> ConvertedConnector {
    let mut outcomes = outcomes(source, true);
    assert_eq!(outcomes.len(), 1_usize);
    match outcomes.remove(0) {
        ConnectorOutcome::Converted(converted) => converted,
        ConnectorOutcome::Failed(error) => panic!("expected a converted connector, got {error:?}"),
    }
}

fn failure(source: &str) -> ConnectorConversionError {
    let mut outcomes = outcomes(source, true);
    assert_eq!(outcomes.len(), 1_usize);
    match outcomes.remove(0) {
        ConnectorOutcome::Failed(error) => error,
        ConnectorOutcome::Converted(converted) => {
            panic!("expected a failed connector, got {}", converted.name)
        }
    }
}

fn auth_value(request: &NewMCPServerRequest) -> Option<String> {
    let credentials = request.credentials.as_ref()?;
    let value = credentials.auth_value.as_ref()?;
    let secret = value.as_ref()?;
    Some(secret.0.expose_secret().to_owned())
}

fn entry(url: &str, extra: &str) -> String {
    format!(r#"{{"mcpServers":{{"test-server":{{"url":"{url}"{extra}}}}}}}"#)
}

#[test]
fn sanitizing_replaces_unsupported_characters_and_collapses_underscores() {
    assert_eq!(sanitize_connector_name(" my  srv-1!! "), "my_srv_1");
    assert_eq!(sanitize_connector_name("__a__b__"), "a_b");
    assert_eq!(sanitize_connector_name("a/b"), "a_b");
    assert_eq!(sanitize_connector_name("na\u{ef}ve"), "na_ve");
    assert_eq!(sanitize_connector_name("A1_b"), "A1_b");
    assert_eq!(sanitize_connector_name(""), "");
}

#[test]
fn a_mapping_entry_is_converted_under_its_key() {
    let converted = converted(&entry("https://mcp.test", r#","type":"sse""#));
    assert_eq!(converted.name, "test-server");
    let request = converted.request;
    assert_eq!(request.server_name.as_deref(), Some("test_server"));
    assert_eq!(request.alias.as_deref(), Some("test_server"));
    assert_eq!(request.transport, MCPTransport::Sse);
    assert_eq!(request.url.as_deref(), Some("https://mcp.test"));
    assert_eq!(request.approval_status.as_deref(), Some("active"));
}

#[rstest]
#[case::sse(r#","type":"sse""#, MCPTransport::Sse)]
#[case::uppercase_sse(r#","type":"SSE""#, MCPTransport::Sse)]
#[case::http(r#","type":"http""#, MCPTransport::Http)]
#[case::url(r#","type":"url""#, MCPTransport::Http)]
#[case::streamable_http(r#","type":"streamable_http""#, MCPTransport::Http)]
#[case::streamable_hyphen(r#","type":"streamable-http""#, MCPTransport::Http)]
#[case::empty_type(r#","type":"""#, MCPTransport::Http)]
#[case::missing_type("", MCPTransport::Http)]
fn the_type_selects_the_transport(#[case] extra: &str, #[case] transport: MCPTransport) {
    let converted = converted(&entry("https://mcp.test", extra));
    assert_eq!(converted.request.transport, transport, "{extra}");
}

#[test]
fn an_unknown_type_prints_the_type_as_sent() {
    let failure = failure(&entry("https://mcp.test", r#","type":"WebSocket""#));
    assert_eq!(failure.name, "test-server");
    assert_eq!(failure.error, "Unsupported connector type 'WebSocket'.");
}

#[test]
fn a_bearer_authorization_header_becomes_a_bearer_token() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","headers":{"Authorization":"Bearer tok","X-Extra":"1"}"#,
    ));
    assert_eq!(converted.request.auth_type, Some(MCPAuth::BearerToken));
    assert_eq!(auth_value(&converted.request).as_deref(), Some("tok"));
    let headers = converted.request.static_headers.unwrap();
    assert_eq!(headers.len(), 1_usize);
    assert_eq!(headers.get("X-Extra").map(String::as_str), Some("1"));
}

#[test]
fn a_non_bearer_authorization_header_keeps_the_whole_value() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","headers":{"Authorization":"Basic abc"}"#,
    ));
    assert_eq!(converted.request.auth_type, Some(MCPAuth::Authorization));
    assert_eq!(auth_value(&converted.request).as_deref(), Some("Basic abc"));
    assert_eq!(converted.request.static_headers, None);
}

#[test]
fn bearer_detection_is_case_insensitive_and_may_leave_an_empty_token() {
    let empty_token = converted(&entry(
        "https://mcp.test",
        r#","headers":{"Authorization":"BEARER "}"#,
    ));
    assert_eq!(empty_token.request.auth_type, Some(MCPAuth::BearerToken));
    assert_eq!(auth_value(&empty_token.request).as_deref(), Some(""));
    let scheme = converted(&entry(
        "https://mcp.test",
        r#","headers":{"Authorization":"BearerX"}"#,
    ));
    assert_eq!(scheme.request.auth_type, Some(MCPAuth::Authorization));
    assert_eq!(auth_value(&scheme.request).as_deref(), Some("BearerX"));
}

#[test]
fn an_authorization_token_wins_over_a_header() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","authorization_token":"direct","headers":{"Authorization":"Bearer header"}"#,
    ));
    assert_eq!(converted.request.auth_type, Some(MCPAuth::BearerToken));
    assert_eq!(auth_value(&converted.request).as_deref(), Some("direct"));
}

#[test]
fn an_empty_authorization_token_falls_back_to_the_header() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","authorization_token":"","headers":{"Authorization":"Bearer header"}"#,
    ));
    assert_eq!(auth_value(&converted.request).as_deref(), Some("header"));
}

#[test]
fn an_empty_authorization_header_means_no_auth() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","headers":{"Authorization":"","X-Extra":"1"}"#,
    ));
    assert_eq!(converted.request.auth_type, Some(MCPAuth::None));
    assert!(converted.request.credentials.is_none());
    let headers = converted.request.static_headers.unwrap();
    assert_eq!(headers.len(), 1_usize);
    assert_eq!(headers.get("X-Extra").map(String::as_str), Some("1"));
}

#[test]
fn every_authorization_header_is_removed_but_only_the_first_value_is_read() {
    let converted = converted(&entry(
        "https://mcp.test",
        r#","headers":{"authorization":"Bearer first","AUTHORIZATION":"Bearer second","X":"1"}"#,
    ));
    assert_eq!(auth_value(&converted.request).as_deref(), Some("first"));
    let headers = converted.request.static_headers.unwrap();
    assert_eq!(headers.len(), 1_usize);
    assert_eq!(headers.get("X").map(String::as_str), Some("1"));
}

#[test]
fn an_entry_without_auth_has_no_credentials() {
    let converted = converted(&entry("https://mcp.test", ""));
    assert_eq!(converted.request.auth_type, Some(MCPAuth::None));
    assert!(converted.request.credentials.is_none());
    assert_eq!(converted.request.static_headers, None);
}

#[test]
fn a_stdio_entry_builds_a_stdio_request() {
    let converted = converted(
        r#"{"mcpServers":{"test-stdio":{"command":"npx","args":["-y","pkg"],"env":{"K":"V"},"description":"d"}}}"#,
    );
    assert_eq!(converted.name, "test-stdio");
    let request = converted.request;
    assert_eq!(request.server_name.as_deref(), Some("test_stdio"));
    assert_eq!(request.alias.as_deref(), Some("test_stdio"));
    assert_eq!(request.transport, MCPTransport::Stdio);
    assert_eq!(request.command.as_deref(), Some("npx"));
    assert_eq!(request.args, ["-y", "pkg"]);
    let env: &OrderedMap = &request.env;
    assert_eq!(env.get("K").map(String::as_str), Some("V"));
    assert_eq!(request.description.as_deref(), Some("d"));
    assert_eq!(request.url, None);
    assert_eq!(request.auth_type, None);
    assert_eq!(request.approval_status.as_deref(), Some("active"));
}

#[test]
fn a_gated_stdio_entry_fails_with_the_pydantic_prefix() {
    let source = r#"{"mcpServers":{"test-stdio":{"command":"npx","args":["-y"]}}}"#;
    let mut outcomes = outcomes(source, false);
    let ConnectorOutcome::Failed(error) = outcomes.remove(0) else {
        panic!("expected a failed connector");
    };
    assert_eq!(
        error.error,
        "Value error, stdio MCP servers are disabled on this proxy. Set LITELLM_ENABLE_MCP_STDIO=true on the proxy and restart to enable them"
    );
}

#[test]
fn an_unknown_stdio_command_fails_through_the_request_validator() {
    let mut outcomes = outcomes(
        r#"{"mcpServers":{"test-stdio":{"command":"bash","args":["-c","x"]}}}"#,
        true,
    );
    let ConnectorOutcome::Failed(error) = outcomes.remove(0) else {
        panic!("expected a failed connector");
    };
    assert_eq!(
        error.error,
        "Value error, Command 'bash' is not in the allowed commands list for stdio transport. Allowed commands: ['node', 'npx']"
    );
}

#[test]
fn an_entry_with_both_a_url_and_a_command_fails() {
    let failure = failure(&entry("https://mcp.test", r#","command":"npx""#));
    assert_eq!(
        failure.error,
        "Connector cannot have both a url and a command."
    );
}

#[test]
fn an_entry_with_neither_a_url_nor_a_command_fails() {
    let failure = failure(r#"{"mcpServers":{"test-server":{"type":"sse"}}}"#);
    assert_eq!(
        failure.error,
        "Connector must have either a url or a command."
    );
}

#[test]
fn a_name_that_sanitizes_to_nothing_fails() {
    let failure = failure(r#"{"mcpServers":{"!!!":{"url":"https://mcp.test"}}}"#);
    assert_eq!(failure.name, "!!!");
    assert_eq!(failure.error, "Connector name is empty after sanitization.");
}

#[test]
fn an_empty_command_with_a_url_is_a_remote_entry() {
    let converted = converted(&entry("https://mcp.test", r#","command":"""#));
    assert_eq!(converted.request.transport, MCPTransport::Http);
}

#[test]
fn an_empty_url_with_a_command_is_a_stdio_entry() {
    let converted =
        converted(r#"{"mcpServers":{"test-server":{"url":"","command":"npx","args":["x"]}}}"#);
    assert_eq!(converted.request.transport, MCPTransport::Stdio);
}

#[test]
fn list_entries_without_names_fail_with_the_url_or_the_index() {
    let outcomes = outcomes(
        r#"{"mcp_servers":[{"url":"https://first"},{"url":""},{"command":"node","args":["x"]}]}"#,
        true,
    );
    let names: Vec<(String, String)> = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            ConnectorOutcome::Failed(error) => (error.name, error.error),
            ConnectorOutcome::Converted(converted) => {
                panic!("expected a failed connector, got {}", converted.name)
            }
        })
        .collect();
    assert_eq!(names.len(), 3_usize);
    assert_eq!(names[0].0, "https://first");
    assert_eq!(names[1].0, "entry 1");
    assert_eq!(names[2].0, "entry 2");
    let errors: Vec<&str> = names.iter().map(|(_, error)| error.as_str()).collect();
    assert_eq!(
        errors,
        ["Connector entries in list form must have a name."; 3]
    );
}

#[test]
fn a_list_entry_with_a_name_keeps_its_raw_name() {
    let converted = converted(r#"{"mcp_servers":[{"name":"my name","url":"https://mcp.test"}]}"#);
    assert_eq!(converted.name, "my name");
    assert_eq!(converted.request.server_name.as_deref(), Some("my_name"));
}

#[test]
fn mapping_outcomes_keep_entry_order() {
    let outcomes = outcomes(
        r#"{"mcpServers":{"zulu":{"url":"https://z"},"alpha":{"url":"https://a"}}}"#,
        true,
    );
    let names: Vec<String> = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            ConnectorOutcome::Converted(converted) => converted.name,
            ConnectorOutcome::Failed(error) => error.name,
        })
        .collect();
    assert_eq!(names, ["zulu", "alpha"]);
}
