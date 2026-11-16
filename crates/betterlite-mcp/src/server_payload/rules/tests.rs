#![cfg(test)]

use std::collections::BTreeSet;
use std::io;
use std::str::FromStr;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::types::IdentifierField;
use betterlite_prompts::arguments::json::Json;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::fields::NewServerField;
use super::{
    MCP_STDIO_DISABLED_MESSAGE, PayloadRules, StdioPolicy, does_mcp_server_exist,
    identifier_conflict_message, is_mcp_stdio_blocked, stamp_omitted_oauth2_flow,
    validate_required_fields, warn_if_id_jag_server_outruns_sso, warn_if_mcp_stdio_blocked,
};
use crate::server_payload::request::NewMCPServerRequest;
use crate::server_record::OAuth2Flow;

fn policy(enabled: bool) -> StdioPolicy {
    StdioPolicy {
        enabled,
        allowed_commands: ["npx".to_owned()].into_iter().collect(),
    }
}

fn payload(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn request(source: &str) -> NewMCPServerRequest {
    NewMCPServerRequest::from_value(payload(source), &policy(false)).unwrap()
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[test]
fn stdio_blocking_needs_stdio_and_a_disabled_policy() {
    assert!(is_mcp_stdio_blocked(Some("stdio"), &policy(false)));
    assert!(!is_mcp_stdio_blocked(Some("stdio"), &policy(true)));
    assert!(!is_mcp_stdio_blocked(Some("http"), &policy(false)));
    assert!(!is_mcp_stdio_blocked(Some("sse"), &policy(false)));
    assert!(!is_mcp_stdio_blocked(None, &policy(false)));
}

#[test]
fn a_blocked_stdio_server_logs_the_disabled_message_once() {
    let (captured, _guard) = capture();
    warn_if_mcp_stdio_blocked(Some("test-server"), Some("stdio"), &policy(false));
    let text = captured.text();
    assert!(text.contains(&format!(
        "MCP server 'test-server' will not start: {MCP_STDIO_DISABLED_MESSAGE}"
    )));
    assert_eq!(text.matches(MCP_STDIO_DISABLED_MESSAGE).count(), 1_usize);
}

#[test]
fn an_unblocked_stdio_server_logs_nothing() {
    let (captured, _guard) = capture();
    warn_if_mcp_stdio_blocked(Some("test-server"), Some("stdio"), &policy(true));
    warn_if_mcp_stdio_blocked(Some("test-server"), Some("http"), &policy(false));
    warn_if_mcp_stdio_blocked(None, None, &policy(false));
    assert!(captured.text().is_empty());
}

#[test]
fn stamping_fills_the_machine_to_machine_flow() {
    let mut machine = request(
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","credentials":{"client_id":"id","client_secret":"secret"}}"#,
    );
    stamp_omitted_oauth2_flow(&mut machine);
    assert_eq!(machine.oauth2_flow, Some(OAuth2Flow::ClientCredentials));
}

#[test]
fn stamping_keeps_an_explicit_flow() {
    let mut explicit = request(
        r#"{"transport":"http","url":"u","auth_type":"oauth2","oauth2_flow":"authorization_code","token_url":"https://token","credentials":{"client_id":"id","client_secret":"secret"}}"#,
    );
    stamp_omitted_oauth2_flow(&mut explicit);
    assert_eq!(explicit.oauth2_flow, Some(OAuth2Flow::AuthorizationCode));
}

#[test]
fn stamping_without_the_machine_to_machine_shape_uses_authorization_code() {
    for source in [
        r#"{"transport":"http","url":"u","auth_type":"oauth2","credentials":{"client_id":"id","client_secret":"secret"}}"#,
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","credentials":{"client_secret":"secret"}}"#,
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","credentials":{"client_id":"id"}}"#,
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","credentials":{"client_id":"","client_secret":"secret"}}"#,
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","credentials":{"client_id":"id","client_secret":""}}"#,
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","authorization_url":"https://authorize","credentials":{"client_id":"id","client_secret":"secret"}}"#,
    ] {
        let mut payload = request(source);
        stamp_omitted_oauth2_flow(&mut payload);
        assert_eq!(payload.oauth2_flow, Some(OAuth2Flow::AuthorizationCode));
    }
}

#[test]
fn stamping_leaves_other_auth_types_alone() {
    let mut payload = request(
        r#"{"transport":"http","url":"u","auth_type":"none","token_url":"https://token","credentials":{"client_id":"id","client_secret":"secret"}}"#,
    );
    stamp_omitted_oauth2_flow(&mut payload);
    assert_eq!(payload.oauth2_flow, None);
}

#[test]
fn stamping_uses_client_credentials_even_when_delegating() {
    let mut payload = request(
        r#"{"transport":"http","url":"u","auth_type":"oauth2","token_url":"https://token","delegate_auth_to_upstream":true,"credentials":{"client_id":"id","client_secret":"secret"}}"#,
    );
    stamp_omitted_oauth2_flow(&mut payload);
    assert_eq!(payload.oauth2_flow, Some(OAuth2Flow::ClientCredentials));
}

#[test]
fn required_fields_follow_python_presence() {
    let payload = request(
        r#"{"transport":"http","url":"u","server_name":"","alias":"","args":[],"mcp_access_groups":[],"allowed_tools":[],"byok_description":[],"tool_name_to_display_name":{},"env":{},"credentials":{},"allow_all_keys":false,"timeout":0,"max_concurrent_requests":0,"auth_type":"none"}"#,
    );
    let required = [
        "server_name",
        "alias",
        "args",
        "mcp_access_groups",
        "allowed_tools",
        "byok_description",
        "tool_name_to_display_name",
        "env",
        "credentials",
        "allow_all_keys",
        "timeout",
        "max_concurrent_requests",
        "auth_type",
    ]
    .map(str::to_owned);
    let error = validate_required_fields(&payload, &required).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Submission is missing required fields: ['server_name', 'alias', 'args', 'mcp_access_groups', 'allowed_tools', 'byok_description', 'auth_type']. Configure required fields via general_settings.mcp_required_fields."
    );
}

#[test]
fn required_fields_report_unknown_names_first() {
    let payload = request(r#"{"transport":"http","url":"u"}"#);
    let required = ["bogus", "server_name", "other"].map(str::to_owned);
    let error = validate_required_fields(&payload, &required).unwrap_err();
    assert_eq!(
        error.to_string(),
        "mcp_required_fields contains unknown field names: ['bogus', 'other']. Check general_settings.mcp_required_fields in your proxy config."
    );
}

#[test]
fn required_fields_keep_input_order_and_duplicates() {
    let payload = request(r#"{"transport":"http","url":"u"}"#);
    let required = ["server_name", "env", "server_name"].map(str::to_owned);
    assert_eq!(
        validate_required_fields(&payload, &required)
            .unwrap_err()
            .to_string(),
        "Submission is missing required fields: ['server_name', 'server_name']. Configure required fields via general_settings.mcp_required_fields."
    );
}

#[test]
fn an_empty_required_list_passes() {
    let payload = request(r#"{"transport":"http","url":"u"}"#);
    assert!(validate_required_fields(&payload, &[]).is_ok());
}

#[test]
fn every_litellm_field_name_parses() {
    for name in [
        "server_id",
        "server_name",
        "alias",
        "description",
        "transport",
        "auth_type",
        "credentials",
        "url",
        "spec_path",
        "mcp_info",
        "mcp_access_groups",
        "allowed_tools",
        "tool_name_to_display_name",
        "tool_name_to_description",
        "extra_headers",
        "static_headers",
        "env_vars",
        "instructions",
        "command",
        "args",
        "env",
        "issuer",
        "authorization_url",
        "token_url",
        "registration_url",
        "oauth2_flow",
        "token_exchange_endpoint",
        "audience",
        "subject_token_type",
        "token_exchange_profile",
        "allow_all_keys",
        "available_on_public_internet",
        "delegate_auth_to_upstream",
        "oauth_passthrough",
        "dcr_bridge",
        "per_server_oauth_discovery",
        "is_byok",
        "byok_description",
        "byok_api_key_help_url",
        "source_url",
        "timeout",
        "max_concurrent_requests",
        "approval_status",
        "submitted_by",
        "submitted_at",
    ] {
        assert!(NewServerField::from_str(name).is_ok(), "missing {name}");
    }
}

#[test]
fn a_payload_rules_value_carries_both_knobs() {
    let rules = PayloadRules {
        prefix_separator: "::".to_owned(),
        stdio: policy(true),
    };
    assert_eq!(rules.prefix_separator, "::");
    assert!(rules.stdio.enabled);
    assert_eq!(
        rules.stdio.allowed_commands,
        BTreeSet::from(["npx".to_owned()])
    );
}

#[test]
fn identifier_conflict_messages_name_the_field() {
    let alias = McpIdentifierConflict {
        field: IdentifierField::Alias,
        value: "taken".to_owned(),
        server_id: "srv-1".to_owned(),
    };
    assert_eq!(
        identifier_conflict_message(&alias),
        "An MCP server with alias 'taken' already exists (server_id=srv-1). MCP server names and aliases must be unique, case-insensitive."
    );
    let name = McpIdentifierConflict {
        field: IdentifierField::ServerName,
        value: "taken".to_owned(),
        server_id: "srv-1".to_owned(),
    };
    assert!(identifier_conflict_message(&name).contains("with server_name 'taken'"));
}

#[test]
fn an_id_jag_warning_needs_the_auth_type_and_a_gap() {
    let (captured, _guard) = capture();
    warn_if_id_jag_server_outruns_sso(
        Some("srv-1"),
        Some("oauth2_id_jag"),
        Some("the SSO token carries no assertion"),
    );
    warn_if_id_jag_server_outruns_sso(
        Some("srv-1"),
        Some("oauth2"),
        Some("the SSO token carries no assertion"),
    );
    warn_if_id_jag_server_outruns_sso(Some("srv-1"), Some("oauth2_id_jag"), None);
    warn_if_id_jag_server_outruns_sso(None, Some("oauth2_id_jag"), Some("gap"));
    let text = captured.text();
    assert!(text.contains(
        "MCP server srv-1 is registered with auth_type=oauth2_id_jag, but the SSO token carries no assertion."
    ));
    assert!(text.contains("MCP server None is registered with auth_type=oauth2_id_jag, but gap."));
    assert_eq!(text.matches("auth_type=oauth2_id_jag").count(), 2_usize);
}

#[test]
fn server_existence_searches_every_id() {
    let ids = ["a", "b", "c"];
    assert!(does_mcp_server_exist(ids, "b"));
    assert!(!does_mcp_server_exist(ids, "z"));
    assert!(!does_mcp_server_exist(std::iter::empty::<&str>(), "b"));
    let owned = vec!["a".to_owned(), "b".to_owned()];
    assert!(does_mcp_server_exist(owned.iter().map(String::as_str), "b"));
}
