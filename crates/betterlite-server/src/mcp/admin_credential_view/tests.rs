#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;

use super::{MCP_ADMIN_CONFIG_CREDENTIAL_KEYS, redact_mcp_credentials};

fn row(source: &str) -> McpServerTable {
    serde_json::from_str(source).unwrap()
}

fn server(source: &str) -> MCPServer {
    serde_json::from_str(source).unwrap()
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn registry(public_ids: &[String], strict: bool, db_servers: Vec<MCPServer>) -> ServerRegistry {
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: public_ids.to_vec(),
            public_hub_strict: strict,
            internal_ip_ranges: None,
        },
        Vec::new(),
        db_servers,
    )
}

fn redacted_row(
    source: &str,
    public_ids: &[&str],
    strict: bool,
    db_servers: Vec<MCPServer>,
) -> McpServerTable {
    let published = ids(public_ids);
    let built = registry(&published, strict, db_servers);
    redact_mcp_credentials(&row(source), &built, &published).unwrap()
}

fn info_fields(view: &McpServerTable) -> BTreeMap<String, JsonValue> {
    let stored = view
        .mcp_info
        .as_ref()
        .expect("the view always carries info");
    serde_json::from_str(&serde_json::to_string(stored).unwrap()).unwrap()
}

#[test]
fn credentials_secrets_dropped_and_admin_config_kept() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-1",
            "transport": "http",
            "credentials": {
                "auth_value": "test-secret",
                "client_secret": "test-client-secret",
                "aws_access_key_id": "test-aws-key",
                "aws_secret_access_key": "test-aws-secret",
                "aws_session_token": "test-aws-session",
                "client_private_key": "test-private-key",
                "upstream_resource": "https://mcp.example.test/resource",
                "upstream_token_header": "X-Upstream-Token",
                "scopes": ["read", "write"]
            }
        }"#,
        &[],
        true,
        Vec::new(),
    );
    let credentials = view.credentials.expect("the admin config survives");
    assert_eq!(
        credentials.upstream_resource,
        Some(Some("https://mcp.example.test/resource".to_owned()))
    );
    assert_eq!(
        credentials.upstream_token_header,
        Some(Some("X-Upstream-Token".to_owned()))
    );
    assert_eq!(
        credentials.scopes,
        Some(Some(vec!["read".to_owned(), "write".to_owned()]))
    );
    assert!(credentials.auth_value.is_none());
    assert!(credentials.client_secret.is_none());
    assert!(credentials.aws_access_key_id.is_none());
    assert!(credentials.aws_secret_access_key.is_none());
    assert!(credentials.aws_session_token.is_none());
    assert!(credentials.client_private_key.is_none());
}

#[test]
fn credentials_empty_upstream_value_dropped() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-2",
            "transport": "http",
            "credentials": {"upstream_resource": "", "upstream_token_header": "X-Upstream-Token"}
        }"#,
        &[],
        true,
        Vec::new(),
    );
    let credentials = view.credentials.expect("the header survives");
    assert_eq!(credentials.upstream_resource, None);
    assert_eq!(
        credentials.upstream_token_header,
        Some(Some("X-Upstream-Token".to_owned()))
    );
}

#[test]
fn credentials_scopes_with_empty_entry_dropped() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-3",
            "transport": "http",
            "credentials": {"upstream_resource": "https://mcp.example.test/resource", "scopes": ["read", ""]}
        }"#,
        &[],
        true,
        Vec::new(),
    );
    let credentials = view.credentials.expect("the resource survives");
    assert_eq!(credentials.scopes, None);
    assert_eq!(
        credentials.upstream_resource,
        Some(Some("https://mcp.example.test/resource".to_owned()))
    );
}

#[test]
fn credentials_empty_scopes_dropped() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-4",
            "transport": "http",
            "credentials": {"upstream_token_header": "X-Upstream-Token", "scopes": []}
        }"#,
        &[],
        true,
        Vec::new(),
    );
    let credentials = view.credentials.expect("the header survives");
    assert_eq!(credentials.scopes, None);
}

#[test]
fn credentials_only_secrets_gives_none() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-5",
            "transport": "http",
            "credentials": {"auth_value": "test-secret", "client_id": "test-client", "upstream_resource": null}
        }"#,
        &[],
        true,
        Vec::new(),
    );
    assert!(view.credentials.is_none());
}

#[test]
fn credentials_absent_gives_none() {
    let view = redacted_row(
        r#"{"server_id": "test-srv-6", "transport": "http"}"#,
        &[],
        true,
        Vec::new(),
    );
    assert!(view.credentials.is_none());
}

#[test]
fn credentials_serialized_keys_are_admin_config_and_scopes() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-7",
            "transport": "http",
            "credentials": {
                "auth_value": "test-secret",
                "client_secret": "test-client-secret",
                "upstream_resource": "https://mcp.example.test/resource",
                "upstream_token_header": "X-Upstream-Token",
                "scopes": ["read"]
            }
        }"#,
        &[],
        true,
        Vec::new(),
    );
    let encoded = serde_json::to_string(&view.credentials).unwrap();
    let fields: BTreeMap<String, JsonValue> = serde_json::from_str(&encoded).unwrap();
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    let mut allowed: Vec<&str> = MCP_ADMIN_CONFIG_CREDENTIAL_KEYS.to_vec();
    allowed.push("scopes");
    allowed.sort_unstable();
    assert_eq!(keys, allowed);
}

#[test]
fn credentials_other_row_fields_unchanged() {
    let source = r#"{
        "server_id": "test-srv-8",
        "server_name": "Test Server",
        "transport": "sse",
        "url": "https://mcp.example.test/sse",
        "instructions": "test instructions",
        "teams": [{"team_id": "test-team"}],
        "mcp_access_groups": ["test-group"],
        "allowed_tools": ["test-tool"],
        "extra_headers": ["X-Test: value"],
        "credentials": {"auth_value": "test-secret", "upstream_resource": "https://mcp.example.test/resource"},
        "mcp_info": {"protocol_version": "2025-06-18", "owner": "test-admin"}
    }"#;
    let view = redacted_row(source, &[], true, Vec::new());
    let mut before: BTreeMap<String, JsonValue> =
        serde_json::from_str(&serde_json::to_string(&row(source)).unwrap()).unwrap();
    let mut after: BTreeMap<String, JsonValue> =
        serde_json::from_str(&serde_json::to_string(&view).unwrap()).unwrap();
    assert!(before.remove("credentials").is_some());
    assert!(before.remove("mcp_info").is_some());
    assert!(after.remove("credentials").is_some());
    assert!(after.remove("mcp_info").is_some());
    assert_eq!(before, after);
}

#[test]
fn public_listed_server_with_strict_default() {
    let view = redacted_row(
        r#"{"server_id": "test-srv-9", "transport": "http"}"#,
        &["test-srv-9"],
        true,
        vec![server(
            r#"{"server_id": "test-srv-9", "name": "Test Server", "transport": "http"}"#,
        )],
    );
    let fields = info_fields(&view);
    assert_eq!(fields.get("is_public"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn public_legacy_public_internet_unlisted() {
    let view = redacted_row(
        r#"{"server_id": "test-srv-10", "transport": "http"}"#,
        &[],
        false,
        vec![server(
            r#"{"server_id": "test-srv-10", "name": "Test Server", "transport": "http", "available_on_public_internet": true}"#,
        )],
    );
    let fields = info_fields(&view);
    assert_eq!(fields.get("is_public"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
}

#[test]
fn public_listed_id_not_registered() {
    let view = redacted_row(
        r#"{"server_id": "test-srv-11", "transport": "http"}"#,
        &["test-srv-11"],
        true,
        Vec::new(),
    );
    let fields = info_fields(&view);
    assert_eq!(fields.get("is_public"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
}

#[test]
fn public_row_without_mcp_info() {
    let view = redacted_row(
        r#"{"server_id": "test-srv-12", "transport": "http"}"#,
        &[],
        true,
        Vec::new(),
    );
    let fields = info_fields(&view);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields.get("is_public"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
}

#[test]
fn public_stored_mcp_info_keys_kept() {
    let view = redacted_row(
        r#"{
            "server_id": "test-srv-13",
            "transport": "http",
            "mcp_info": {"protocol_version": "2025-06-18", "owner": "test-admin"}
        }"#,
        &["test-srv-13"],
        true,
        vec![server(
            r#"{"server_id": "test-srv-13", "name": "Test Server", "transport": "http"}"#,
        )],
    );
    let fields = info_fields(&view);
    assert_eq!(
        fields.get("protocol_version"),
        Some(&JsonValue::String("2025-06-18".to_owned()))
    );
    assert_eq!(
        fields.get("owner"),
        Some(&JsonValue::String("test-admin".to_owned()))
    );
    assert_eq!(fields.get("is_public"), Some(&JsonValue::Bool(true)));
}

#[test]
fn public_stored_flags_overwritten() {
    let listed = redacted_row(
        r#"{
            "server_id": "test-srv-14",
            "transport": "http",
            "mcp_info": {"is_public": false, "is_public_explicit": true}
        }"#,
        &["test-srv-14"],
        true,
        vec![server(
            r#"{"server_id": "test-srv-14", "name": "Test Server", "transport": "http"}"#,
        )],
    );
    let listed_fields = info_fields(&listed);
    assert_eq!(listed_fields.get("is_public"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        listed_fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(true))
    );
    let unlisted = redacted_row(
        r#"{
            "server_id": "test-srv-15",
            "transport": "http",
            "mcp_info": {"is_public": true, "is_public_explicit": true}
        }"#,
        &[],
        true,
        vec![server(
            r#"{"server_id": "test-srv-15", "name": "Test Server", "transport": "http"}"#,
        )],
    );
    let unlisted_fields = info_fields(&unlisted);
    assert_eq!(
        unlisted_fields.get("is_public"),
        Some(&JsonValue::Bool(false))
    );
    assert_eq!(
        unlisted_fields.get("is_public_explicit"),
        Some(&JsonValue::Bool(false))
    );
}
