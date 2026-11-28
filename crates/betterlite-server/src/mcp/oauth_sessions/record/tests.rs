#![cfg(test)]

use betterlite_db::managed_resource_rows::mcp::McpStatus;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use super::{RecordError, build_temporary_record};
use crate::mcp::oauth_sessions::tests::testing::{now, request};

const FULL_SESSION: &str = r#"{
    "server_id":"test-session-1",
    "server_name":"Test Session",
    "alias":"session-alias",
    "description":"A session",
    "url":"https://mcp.example.test/sse",
    "transport":"sse",
    "auth_type":"oauth2",
    "credentials":{"client_id":"abc"},
    "mcp_access_groups":["group-a"],
    "allowed_tools":["tool-a"],
    "extra_headers":["X-Tenant: acme"],
    "static_headers":{"X-Api-Key":"header-secret"},
    "args":["--flag"],
    "env":{"TOKEN":"env-secret"},
    "issuer":"https://idp.example.test",
    "authorization_url":"https://idp.example.test/authorize",
    "token_url":"https://idp.example.test/token",
    "registration_url":"https://idp.example.test/register",
    "allow_all_keys":true,
    "available_on_public_internet":false,
    "timeout":30.0,
    "max_concurrent_requests":7
}"#;

#[rstest]
#[case::server_name(
    r#"{"server_id":"test-session-1","server_name":"named","alias":"aliased"}"#,
    "named"
)]
#[case::alias(r#"{"server_id":"test-session-1","alias":"aliased"}"#, "aliased")]
#[case::empty_server_name(
    r#"{"server_id":"test-session-1","server_name":"","alias":"aliased"}"#,
    "aliased"
)]
#[case::server_id(r#"{"server_id":"test-session-1","alias":""}"#, "test-session-1")]
fn the_server_name_falls_back_from_name_to_alias_to_id(#[case] body: &str, #[case] expected: &str) {
    let record = build_temporary_record(&request(body), None, "test-session-1", now())
        .expect("the record builds");
    assert_eq!(record.server_name.as_deref(), Some(expected));
}

#[test]
fn the_record_copies_every_string_field_the_session_keeps() {
    let record = build_temporary_record(
        &request(FULL_SESSION),
        Some("admin-1"),
        "test-session-1",
        now(),
    )
    .expect("the record builds");
    assert_eq!(record.server_name.as_deref(), Some("Test Session"));
    assert_eq!(record.alias.as_deref(), Some("session-alias"));
    assert_eq!(record.description.as_deref(), Some("A session"));
    assert_eq!(record.url.as_deref(), Some("https://mcp.example.test/sse"));
    assert_eq!(record.transport, MCPTransport::Sse);
    assert_eq!(record.auth_type, Some(MCPAuth::Oauth2));
    let credentials = record.credentials.as_ref().expect("the credentials copy");
    assert_eq!(credentials.client_id, Some(Some("abc".to_owned())));
    assert_eq!(record.mcp_access_groups, vec!["group-a".to_owned()]);
}

#[test]
fn the_record_copies_the_tool_lists_and_the_oauth_urls() {
    let record = build_temporary_record(
        &request(FULL_SESSION),
        Some("admin-1"),
        "test-session-1",
        now(),
    )
    .expect("the record builds");
    assert_eq!(record.allowed_tools, vec!["tool-a".to_owned()]);
    assert_eq!(record.extra_headers, vec!["X-Tenant: acme".to_owned()]);
    assert_eq!(record.args, vec!["--flag".to_owned()]);
    assert_eq!(record.issuer.as_deref(), Some("https://idp.example.test"));
    assert_eq!(
        record.authorization_url.as_deref(),
        Some("https://idp.example.test/authorize")
    );
    assert_eq!(
        record.token_url.as_deref(),
        Some("https://idp.example.test/token")
    );
    assert_eq!(
        record.registration_url.as_deref(),
        Some("https://idp.example.test/register")
    );
}

#[test]
fn the_record_copies_the_flagged_and_numeric_fields() {
    let record = build_temporary_record(
        &request(FULL_SESSION),
        Some("admin-1"),
        "test-session-1",
        now(),
    )
    .expect("the record builds");
    assert!(record.allow_all_keys.0);
    assert!(!record.available_on_public_internet.0);
    assert_eq!(record.max_concurrent_requests, Some(7));
    assert_eq!(
        record.timeout,
        Some(Decimal::try_from(30.0_f64).expect("30 is a decimal"))
    );
}

#[test]
fn env_and_static_headers_arrive_as_secret_maps() {
    let record = build_temporary_record(
        &request(FULL_SESSION),
        Some("admin-1"),
        "test-session-1",
        now(),
    )
    .expect("the record builds");
    let token = record.env.0.get("TOKEN").expect("the env entry is copied");
    assert_eq!(token.expose_secret(), "env-secret");
    let headers = record
        .static_headers
        .as_ref()
        .expect("the headers are copied");
    let header = headers
        .0
        .get("X-Api-Key")
        .expect("the header entry is copied");
    assert_eq!(header.expose_secret(), "header-secret");
}

#[test]
fn a_timeout_is_absent_when_the_request_gives_none() {
    let body =
        r#"{"server_id":"test-session-1","url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let record = build_temporary_record(&request(body), None, "test-session-1", now())
        .expect("the record builds");
    assert_eq!(record.timeout, None);
}

#[test]
fn a_timeout_that_cannot_be_a_decimal_is_an_error() {
    let mut payload = request(FULL_SESSION);
    payload.timeout = Some(f64::NAN);
    let error = build_temporary_record(&payload, None, "test-session-1", now())
        .expect_err("a NaN has no decimal form");
    assert!(matches!(error, RecordError::Timeout { .. }));
}

#[test]
fn the_audit_fields_carry_the_creator_and_one_timestamp() {
    let record = build_temporary_record(
        &request(FULL_SESSION),
        Some("admin-1"),
        "test-session-1",
        now(),
    )
    .expect("the record builds");
    assert_eq!(record.created_by.as_deref(), Some("admin-1"));
    assert_eq!(record.updated_by.as_deref(), Some("admin-1"));
    let created = serde_json::to_string(&record.created_at).expect("the timestamp encodes");
    let updated = serde_json::to_string(&record.updated_at).expect("the timestamp encodes");
    assert_eq!(created, updated);
    assert_ne!(created, "null");
}

#[test]
fn a_missing_creator_leaves_the_audit_fields_empty() {
    let record = build_temporary_record(&request(FULL_SESSION), None, "test-session-1", now())
        .expect("the record builds");
    assert_eq!(record.created_by, None);
    assert_eq!(record.updated_by, None);
}

#[test]
fn every_field_the_session_does_not_take_keeps_its_row_default() {
    let record = build_temporary_record(&request(FULL_SESSION), None, "test-session-1", now())
        .expect("the record builds");
    assert!(!record.is_config.0);
    assert!(record.spec_path.is_none());
    assert!(record.instructions.is_none());
    assert!(record.tool_name_to_display_name.is_none());
    assert!(record.tool_name_to_description.is_none());
    assert!(record.pinned_tools.is_none());
    assert!(record.env_vars.is_none());
    assert!(record.oauth2_flow.is_none());
    assert!(record.token_exchange_endpoint.is_none());
    assert!(record.audience.is_none());
    assert!(record.subject_token_type.is_none());
    assert!(record.token_exchange_profile.is_none());
}

#[test]
fn the_byok_and_passthrough_fields_keep_their_row_defaults() {
    let record = build_temporary_record(&request(FULL_SESSION), None, "test-session-1", now())
        .expect("the record builds");
    assert!(!record.delegate_auth_to_upstream.0);
    assert!(!record.oauth_passthrough.0);
    assert!(record.dcr_bridge.is_none());
    assert!(!record.per_server_oauth_discovery.0);
    assert!(!record.is_byok.0);
    assert!(record.byok_description.is_empty());
    assert!(record.byok_api_key_help_url.is_none());
    assert!(record.source_url.is_none());
}

#[test]
fn the_status_and_review_fields_keep_their_row_defaults() {
    let record = build_temporary_record(&request(FULL_SESSION), None, "test-session-1", now())
        .expect("the record builds");
    assert_eq!(record.status, Some(McpStatus::Unknown));
    assert_eq!(record.approval_status.as_deref(), Some("active"));
    assert!(record.submitted_by.is_none());
    assert!(record.submitted_at.is_none());
    assert!(record.reviewed_at.is_none());
    assert!(record.review_notes.is_none());
    assert!(record.teams.is_empty());
}

#[test]
fn an_empty_request_leaves_the_list_fields_empty() {
    let body = r#"{"server_id":"test-session-1"}"#;
    let record = build_temporary_record(&request(body), None, "test-session-1", now())
        .expect("the record builds");
    assert!(record.allowed_tools.is_empty());
    assert!(record.extra_headers.is_empty());
    assert!(record.mcp_access_groups.is_empty());
    assert!(record.args.is_empty());
    assert!(record.env.0.is_empty());
    assert!(record.static_headers.is_none());
    assert!(record.credentials.is_none());
    assert_eq!(record.server_name.as_deref(), Some("test-session-1"));
}
