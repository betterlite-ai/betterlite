#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::connector_import::request::{
    ConnectorImportRequest, ConnectorServers, MCPConnectorImportFailure,
    MCPConnectorImportResponse, MCPConnectorImportResult, MCPConnectorImportSkipped,
};

fn parse(source: &str) -> Result<ConnectorImportRequest, serde_json::Error> {
    ConnectorImportRequest::from_slice(source.as_bytes())
}

fn entries(
    request: ConnectorImportRequest,
) -> Vec<(String, crate::connector_import::request::ConnectorEntry)> {
    let ConnectorServers::Mapping(entries) = request.mcp_servers else {
        panic!("expected a mapping");
    };
    entries.into_iter().collect()
}

#[test]
fn mapping_entries_keep_their_key_order() {
    let parsed =
        entries(parse(r#"{"mcpServers":{"zulu":{"url":"u1"},"alpha":{"url":"u2"}}}"#).unwrap());
    let names: Vec<&str> = parsed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["zulu", "alpha"]);
    assert_eq!(parsed.len(), 2_usize);
    assert_eq!(parsed[0].1.url.as_deref(), Some("u1"));
    assert_eq!(parsed[1].1.url.as_deref(), Some("u2"));
}

#[test]
fn list_entries_keep_their_order() {
    let request = parse(r#"{"mcp_servers":[{"name":"first","url":"u1"},{"url":"u2"}]}"#).unwrap();
    let ConnectorServers::List(entries) = request.mcp_servers else {
        panic!("expected a list");
    };
    assert_eq!(entries.len(), 2_usize);
    assert_eq!(entries[0].name.as_deref(), Some("first"));
    assert_eq!(entries[1].name, None);
    assert_eq!(entries[1].url.as_deref(), Some("u2"));
}

#[test]
fn the_snake_case_and_camel_case_keys_are_both_read() {
    assert!(matches!(
        parse(r#"{"mcp_servers":{}}"#).unwrap().mcp_servers,
        ConnectorServers::Mapping(entries) if entries.is_empty()
    ));
    assert!(matches!(
        parse(r#"{"mcpServers":{}}"#).unwrap().mcp_servers,
        ConnectorServers::Mapping(entries) if entries.is_empty()
    ));
    assert!(matches!(
        parse(r#"{"mcpServers":[]}"#).unwrap().mcp_servers,
        ConnectorServers::List(entries) if entries.is_empty()
    ));
}

#[test]
fn the_first_present_key_wins() {
    let parsed =
        entries(parse(r#"{"mcp_servers":{"snake":{}},"mcpServers":{"camel":{}}}"#).unwrap());
    assert_eq!(parsed.len(), 1_usize);
    assert_eq!(parsed[0].0, "snake");
}

#[test]
fn a_null_snake_key_does_not_fall_back_to_the_camel_key() {
    assert!(parse(r#"{"mcp_servers":null,"mcpServers":{}}"#).is_err());
}

#[rstest]
#[case::missing_key("{}")]
#[case::text_value(r#"{"mcpServers":"text"}"#)]
#[case::number_value(r#"{"mcpServers":5}"#)]
#[case::array_root("[]")]
fn a_missing_or_wrong_shaped_key_fails(#[case] source: &str) {
    assert!(parse(source).is_err(), "{source} should be rejected");
}

#[test]
fn both_authorization_token_spellings_are_read() {
    let snake = entries(
        parse(r#"{"mcpServers":{"a":{"url":"u","authorization_token":"snake"}}}"#).unwrap(),
    );
    assert_eq!(
        snake[0]
            .1
            .authorization_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("snake")
    );
    let camel =
        entries(parse(r#"{"mcpServers":{"a":{"url":"u","authorizationToken":"camel"}}}"#).unwrap());
    assert_eq!(
        camel[0]
            .1
            .authorization_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("camel")
    );
    let both = entries(
        parse(r#"{"mcpServers":{"a":{"url":"u","authorization_token":"snake","authorizationToken":"camel"}}}"#)
            .unwrap(),
    );
    assert_eq!(
        both[0]
            .1
            .authorization_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("snake")
    );
}

#[test]
fn a_null_token_key_holds_its_position() {
    let parsed = entries(
        parse(r#"{"mcpServers":{"a":{"url":"u","authorization_token":null,"authorizationToken":"camel"}}}"#)
            .unwrap(),
    );
    assert!(parsed[0].1.authorization_token.is_none());
}

#[test]
fn an_entry_defaults_args_and_env() {
    let parsed = entries(parse(r#"{"mcpServers":{"a":{}}}"#).unwrap());
    let entry = &parsed[0].1;
    assert!(entry.args.is_empty());
    assert!(entry.env.is_empty());
    assert_eq!(entry.headers, None);
    assert_eq!(entry.command, None);
    assert_eq!(entry.description, None);
    assert_eq!(entry.entry_type, None);
}

#[test]
fn the_type_key_fills_the_entry_type() {
    let parsed = entries(parse(r#"{"mcpServers":{"a":{"type":"sse","url":"u"}}}"#).unwrap());
    assert_eq!(parsed[0].1.entry_type.as_deref(), Some("sse"));
}

#[test]
fn unknown_entry_keys_are_ignored() {
    let parsed = entries(parse(r#"{"mcpServers":{"a":{"url":"u","future":"value"}}}"#).unwrap());
    assert_eq!(parsed[0].1.url.as_deref(), Some("u"));
}

#[test]
fn headers_keep_their_order() {
    let parsed = entries(
        parse(r#"{"mcpServers":{"a":{"url":"u","headers":{"X-B":"2","X-A":"1"}}}}"#).unwrap(),
    );
    let headers = parsed[0].1.headers.as_ref().unwrap();
    let names: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(names, ["X-B", "X-A"]);
}

#[test]
fn headers_and_env_require_string_values() {
    assert!(parse(r#"{"mcpServers":{"a":{"url":"u","headers":{"X":1}}}}"#).is_err());
    assert!(parse(r#"{"mcpServers":{"a":{"url":"u","env":{"K":1}}}}"#).is_err());
}

#[test]
fn args_require_strings() {
    assert!(parse(r#"{"mcpServers":{"a":{"command":"npx","args":[1]}}}"#).is_err());
}

#[test]
fn garbage_bytes_are_an_error() {
    assert!(ConnectorImportRequest::from_slice(b"\xffnot json").is_err());
    assert!(ConnectorImportRequest::from_slice(b"").is_err());
}

#[test]
fn response_models_serialize_in_field_order() {
    let response = MCPConnectorImportResponse {
        imported: vec![MCPConnectorImportResult {
            name: "n".to_owned(),
            server_id: "s".to_owned(),
            alias: "a".to_owned(),
        }],
        skipped: vec![MCPConnectorImportSkipped {
            name: "skip".to_owned(),
            reason: "why".to_owned(),
        }],
        errors: vec![MCPConnectorImportFailure {
            name: "fail".to_owned(),
            error: "because".to_owned(),
        }],
    };
    assert_eq!(
        serde_json::to_string(&response).unwrap(),
        r#"{"imported":[{"name":"n","server_id":"s","alias":"a"}],"skipped":[{"name":"skip","reason":"why"}],"errors":[{"name":"fail","error":"because"}]}"#
    );
}
