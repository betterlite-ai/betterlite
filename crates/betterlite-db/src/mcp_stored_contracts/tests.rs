#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{
    decode::decode_mcp_stored_contract, pinned::parse_pinned_tools,
    pinned::validate_mcp_protocol_metadata, *,
};

fn wire(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

#[test]
fn each_transport() {
    for value in ["sse", "http", "stdio"] {
        let _: MCPTransport = serde_json::from_str(&format!("\"{value}\"")).unwrap();
    }
}

#[test]
fn each_auth_type() {
    for value in [
        "none",
        "api_key",
        "bearer_token",
        "basic",
        "authorization",
        "oauth2",
        "aws_sigv4",
        "token",
        "oauth2_token_exchange",
        "oauth2_id_jag",
        "true_passthrough",
        "oauth_delegate",
    ] {
        let _: MCPAuth = serde_json::from_str(&format!("\"{value}\"")).unwrap();
    }
}

#[test]
fn nullable_auth() {
    assert_eq!(serde_json::from_str::<MCPAuthType>("null").unwrap(), None);
}

#[test]
fn each_token_endpoint_method() {
    for value in ["client_secret_basic", "client_secret_post"] {
        let _: MCPTokenEndpointAuthMethod = serde_json::from_str(&format!("\"{value}\"")).unwrap();
    }
}

#[test]
fn all_fields_optional() {
    assert!(
        serde_json::from_str::<MCPCredentials>("{}")
            .unwrap()
            .auth_value
            .is_none()
    );
}
#[test]
fn explicit_null_preserved() {
    assert!(matches!(
        serde_json::from_str::<MCPCredentials>(r#"{"auth_value":null}"#)
            .unwrap()
            .auth_value,
        Some(None)
    ));
}

#[test]
fn credentials_all_fields() {
    let credentials: MCPCredentials = serde_json::from_str(r#"{"auth_value":"secret", "client_id":"id", "client_secret":"client-secret", "scopes":["a"], "aws_access_key_id":"ak", "aws_secret_access_key":"sk", "aws_session_token":"st", "aws_region_name":"r", "aws_service_name":"s", "aws_role_name":"role", "aws_session_name":"session", "audience":"aud", "token_exchange_endpoint":"endpoint", "id_jag_resource_token_endpoint":"jag", "id_jag_resource":"resource", "upstream_resource":"upstream", "upstream_token_header":"x-token", "client_private_key":"pem", "client_private_key_id":"kid", "client_assertion_signing_alg":"RS256", "token_endpoint_auth_method":"client_secret_post", "redirect_uris":["https://a"], "token_exchange_profile":"rfc8693"}"#).unwrap();
    assert_eq!(
        credentials.auth_value.unwrap().unwrap().expose_secret(),
        "secret"
    );
}

#[test]
fn list_fields() {
    assert_eq!(
        serde_json::from_str::<MCPCredentials>(r#"{"scopes":["a"],"redirect_uris":["b"]}"#)
            .unwrap()
            .scopes
            .unwrap()
            .unwrap(),
        ["a"]
    );
}
#[test]
fn unknown_credential_keys() {
    assert!(
        serde_json::from_str::<MCPCredentials>(r#"{"future":true}"#)
            .unwrap()
            .client_id
            .is_none()
    );
}
#[test]
fn docstring_defaults_not_materialized() {
    assert!(
        serde_json::from_str::<MCPCredentials>("{}")
            .unwrap()
            .client_assertion_signing_alg
            .is_none()
    );
}
#[test]
fn secret_debug_redaction() {
    assert!(
        !format!(
            "{:?}",
            serde_json::from_str::<MCPCredentials>(r#"{"auth_value":"do-not-log"}"#).unwrap()
        )
        .contains("do-not-log")
    );
}

#[test]
fn default_description_schema() {
    let tool: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    assert_eq!(tool.description, "");
    assert!(tool.input_schema.is_empty());
}
#[test]
fn independent_schema_defaults() {
    let first: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    let mut second: PinnedMCPTool = serde_json::from_str("{}").unwrap();
    second.input_schema.insert("x".to_owned(), JsonValue::Null);
    assert!(first.input_schema.is_empty());
}
#[test]
fn falsey_raw_values() {
    for value in ["null", "false", "0", "\"\"", "{}", "[]"] {
        assert!(parse_pinned_tools(wire(value)).unwrap().is_none());
    }
}
#[test]
fn falsey_json_values() {
    for value in ["null", "false", "0", "\"\"", "{}", "[]"] {
        assert!(parse_pinned_tools(wire(value)).unwrap().is_none());
    }
}
#[test]
fn stored_json_tools() {
    assert_eq!(
        parse_pinned_tools(wire(r#"{"weather":{"description":"forecast"}}"#))
            .unwrap()
            .unwrap()["weather"]
            .description,
        "forecast"
    );
}
#[test]
fn nested_schema_preserved() {
    let tools = parse_pinned_tools(wire(
        r#"{"tool":{"input_schema":{"properties":{"x":{"type":"string"}}}}}"#,
    ))
    .unwrap()
    .unwrap();
    assert!(
        serde_json::to_string(&tools)
            .unwrap()
            .contains("\"type\":\"string\"")
    );
}
#[test]
fn whitespace_json_error() {
    parse_pinned_tools(wire(r#""  ""#)).unwrap_err();
}
#[test]
fn nonempty_list_rejected() {
    parse_pinned_tools(wire("[\"tool\"]")).unwrap_err();
}
#[test]
fn tool_extra_field_rejected() {
    parse_pinned_tools(wire(r#"{"tool":{"extra":true}}"#)).unwrap_err();
}
#[test]
fn invalid_tool_field_type() {
    parse_pinned_tools(wire(r#"{"tool":{"description":false}}"#)).unwrap_err();
}

#[test]
fn protocol_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("custom".to_owned(), wire(r#"{"x":1}"#));
    metadata.insert("protocol_version".to_owned(), wire("\"auto\""));
    validate_mcp_protocol_metadata(metadata).unwrap();
}
#[test]
fn absent_version() {
    validate_mcp_protocol_metadata(BTreeMap::new()).unwrap();
}
#[test]
fn null_version_rejected() {
    let mut metadata = BTreeMap::new();
    metadata.insert("protocol_version".to_owned(), JsonValue::Null);
    validate_mcp_protocol_metadata(metadata).unwrap_err();
}
#[test]
fn future_version_rejected() {
    let mut metadata = BTreeMap::new();
    metadata.insert("protocol_version".to_owned(), wire("\"2026-07-28\""));
    validate_mcp_protocol_metadata(metadata).unwrap_err();
}

#[test]
fn mcp_stored_limits() {
    assert!(matches!(
        decode_mcp_stored_contract(&vec![b' '; MAX_STORED_BYTES + 1]),
        Err(decode::McpStoredDecodeError::TooLarge)
    ));
    let deep = format!(
        "{}0{}",
        "[".repeat(MAX_STORED_DEPTH + 2),
        "]".repeat(MAX_STORED_DEPTH + 2)
    );
    assert!(matches!(
        decode_mcp_stored_contract(deep.as_bytes()),
        Err(decode::McpStoredDecodeError::TooDeep | decode::McpStoredDecodeError::Json(_))
    ));
}
#[test]
fn replays_the_mcp_stored_contract_corpus() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mcp_stored_contract");
    for entry in fs::read_dir(path).unwrap() {
        drop(decode_mcp_stored_contract(
            &fs::read(entry.unwrap().path()).unwrap(),
        ));
    }
}
