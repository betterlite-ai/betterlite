#![cfg(test)]

pub(crate) mod support;

use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::{McpEnvVarScope, McpOAuth2Flow};
use betterlite_db::managed_resource_rows::wire::{PublicInternet, RowFlag};
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use crate::registry::lookup::ServerLookups as _;
use crate::registry::permissions::PermissionExpansion as _;
use crate::registry::table::ServerTables as _;
use support::{record, registry, registry_with, settings};

const PRIVATE: &str = r#"{"server_id":"test-a","name":"Alpha","transport":"http","available_on_public_internet":false}"#;
const PUBLIC: &str = r#"{"server_id":"test-b","name":"Beta","transport":"http"}"#;
const PRIVATE_ALIAS: &str = r#"{"server_id":"test-a","name":"Alpha","transport":"http","alias":"test-shared","available_on_public_internet":false}"#;
const PUBLIC_SERVER_NAME: &str =
    r#"{"server_id":"test-b","name":"Beta","transport":"http","server_name":"test-shared"}"#;
const PUBLIC_NAME: &str = r#"{"server_id":"test-c","name":"test-shared","transport":"http"}"#;
const PRIVATE_SERVER_NAME: &str = r#"{"server_id":"test-a","name":"Alpha","transport":"http","server_name":"test-shared","available_on_public_internet":false}"#;
const ALIAS_ALPHA: &str =
    r#"{"server_id":"test-a","name":"Alpha","transport":"http","alias":"test-alias"}"#;
const ALIAS_TWIN: &str =
    r#"{"server_id":"test-b","name":"Beta","transport":"http","alias":"test-alias"}"#;
const NAMED_GAMMA: &str =
    r#"{"server_id":"test-c","name":"Gamma","transport":"http","server_name":"test-gamma"}"#;
const CALLED_GAMMA: &str = r#"{"server_id":"test-d","name":"test-gamma","transport":"http"}"#;
const NAMED_AFTER_ID: &str = r#"{"server_id":"test-e","name":"test-a","transport":"http"}"#;
const FULL_RECORD: &str = r#"{
  "server_id": "test-full",
  "name": "Full Server",
  "alias": "test-full-alias",
  "server_name": "test-full-name",
  "url": "https://mcp.example/mcp",
  "spec_path": "/specs/full.json",
  "transport": "sse",
  "auth_type": "oauth2",
  "instructions": "Use it",
  "mcp_info": {"description": "A full server"},
  "extra_headers": ["X-Extra"],
  "allowed_tools": ["echo", "sum"],
  "tool_name_to_display_name": {"echo": "Echo"},
  "tool_name_to_description": {"echo": "Echoes"},
  "pinned_tools": {"echo": {"description": "Echoes", "input_schema": {}}},
  "static_headers": {"X-Static": "static-secret"},
  "env_vars": [{"name": "EV", "value": "env-value"}],
  "command": "run-server",
  "args": ["--flag"],
  "env": {"ENVKEY": "env-secret"},
  "issuer": "https://issuer.example",
  "authorization_url": "https://live.example/auth",
  "token_url": "https://live.example/token",
  "registration_url": "https://live.example/register",
  "oauth2_flow": "client_credentials",
  "token_exchange_endpoint": "https://exchange.example/token",
  "audience": "aud",
  "subject_token_type": "urn:custom",
  "token_exchange_profile": "custom-profile",
  "access_groups": ["group-a"],
  "allow_all_keys": true,
  "available_on_public_internet": false,
  "delegate_auth_to_upstream": true,
  "oauth_passthrough": true,
  "dcr_bridge": true,
  "per_server_oauth_discovery": true,
  "is_byok": true,
  "byok_description": ["bring your own"],
  "byok_api_key_help_url": "https://help.example",
  "source_url": "https://source.example",
  "created_at": "2026-01-02T03:04:05Z",
  "updated_at": "2026-02-03T04:05:06.5Z",
  "timeout": 2.5,
  "max_concurrent_requests": 7
}"#;
const SCOPED: &str = r#"{"server_id":"test-scoped","name":"Scoped","transport":"http","configured_scopes":["read","write"]}"#;
const EMPTY_SCOPES: &str = r#"{"server_id":"test-empty-scopes","name":"Empty Scopes","transport":"http","configured_scopes":[]}"#;
const NUMERIC_DESCRIPTION: &str = r#"{"server_id":"test-number","name":"Number","transport":"http","mcp_info":{"description":5}}"#;
const CONFIGURED_URLS: &str = r#"{"server_id":"test-urls","name":"Urls","transport":"http","authorization_url":"https://live/auth","configured_authorization_url":"https://configured/auth","token_url":"https://live/token","configured_token_url":"https://configured/token","registration_url":"https://live/register","configured_registration_url":"https://configured/register"}"#;
const LIVE_URLS: &str = r#"{"server_id":"test-live-urls","name":"Live Urls","transport":"http","authorization_url":"https://live/auth","token_url":"https://live/token","registration_url":"https://live/register"}"#;
const EMPTY_CONFIGURED_URL: &str = r#"{"server_id":"test-empty-url","name":"Empty Url","transport":"http","authorization_url":"https://live/auth","configured_authorization_url":""}"#;
const BAD_ENV_VAR: &str = r#"{"server_id":"test-env-var","name":"Env Var","transport":"http","env_vars":[{"value":"no name"},{"name":"GOOD","value":"v"}]}"#;
const CONFIG_BETA: &str =
    r#"{"server_id":"test-b","name":"Beta","transport":"http","url":"https://config.example"}"#;
const DB_BETA: &str =
    r#"{"server_id":"test-b","name":"Beta","transport":"http","url":"https://db.example"}"#;
const DB_ONLY: &str = r#"{"server_id":"test-z","name":"Zulu","transport":"http"}"#;

#[test]
fn lookup_without_ip_reaches_a_private_server() {
    let loaded = registry(&[PRIVATE], &[]);
    assert!(loaded.is_accessible_from_ip(&record(PRIVATE), None));
    assert!(loaded.server_by_id("test-a", None).is_some());
}

#[test]
fn lookup_public_internet_server_passes_an_external_ip() {
    let loaded = registry(&[PUBLIC], &[]);
    assert!(loaded.is_accessible_from_ip(&record(PUBLIC), Some("8.8.8.8")));
    assert!(loaded.server_by_id("test-b", Some("8.8.8.8")).is_some());
}

#[test]
fn lookup_public_list_server_passes_an_external_ip() {
    let loaded = registry_with(settings(&["test-a"], None), &[PRIVATE], &[]);
    assert!(loaded.is_accessible_from_ip(&record(PRIVATE), Some("8.8.8.8")));
    assert!(loaded.server_by_id("test-a", Some("8.8.8.8")).is_some());
}

#[test]
fn lookup_private_server_uses_the_configured_ranges() {
    let loaded = registry_with(settings(&[], Some(&["203.0.113.0/24"])), &[PRIVATE], &[]);
    let private = record(PRIVATE);
    assert!(loaded.is_accessible_from_ip(&private, Some("203.0.113.7")));
    assert!(!loaded.is_accessible_from_ip(&private, Some("10.1.2.3")));
}

#[test]
fn lookup_private_server_falls_back_to_the_default_ranges() {
    let loaded = registry(&[PRIVATE], &[]);
    let private = record(PRIVATE);
    assert!(loaded.is_accessible_from_ip(&private, Some("192.168.1.5")));
    assert!(!loaded.is_accessible_from_ip(&private, Some("8.8.8.8")));
}

#[test]
fn lookup_by_id_blocks_a_server_the_ip_cannot_reach() {
    let loaded = registry_with(settings(&[], Some(&["203.0.113.0/24"])), &[PRIVATE], &[]);
    assert!(loaded.server_by_id("test-a", Some("8.8.8.8")).is_none());
    assert!(loaded.server_by_id("test-a", Some("203.0.113.7")).is_some());
    assert!(
        loaded
            .server_by_id("test-ghost", Some("203.0.113.7"))
            .is_none()
    );
}

#[test]
fn lookup_by_id_prefers_the_database_record_of_a_shared_id() {
    let loaded = registry(&[CONFIG_BETA], &[DB_BETA]);
    let found = loaded.server_by_id("test-b", None).unwrap();
    assert_eq!(found.fields.url.as_deref(), Some("https://db.example"));
}

#[test]
fn lookup_by_name_prefers_an_alias() {
    let loaded = registry(&[PRIVATE_ALIAS, PUBLIC_SERVER_NAME, PUBLIC_NAME], &[]);
    let found = loaded.server_by_name("test-shared", None).unwrap();
    assert_eq!(found.fields.server_id, "test-a");
}

#[test]
fn lookup_by_name_falls_through_to_the_server_name() {
    let loaded = registry(&[PUBLIC_SERVER_NAME, PUBLIC_NAME], &[]);
    let found = loaded.server_by_name("test-shared", None).unwrap();
    assert_eq!(found.fields.server_id, "test-b");
}

#[test]
fn lookup_by_name_falls_through_to_the_name() {
    let loaded = registry(&[PUBLIC_NAME], &[]);
    let found = loaded.server_by_name("test-shared", None).unwrap();
    assert_eq!(found.fields.server_id, "test-c");
}

#[test]
fn lookup_by_name_blocked_alias_match_is_none() {
    let loaded = registry_with(
        settings(&[], Some(&["203.0.113.0/24"])),
        &[PRIVATE_ALIAS, PUBLIC_SERVER_NAME],
        &[],
    );
    assert!(
        loaded
            .server_by_name("test-shared", Some("8.8.8.8"))
            .is_none()
    );
    assert!(
        loaded
            .server_by_name("test-shared", Some("203.0.113.7"))
            .is_some()
    );
}

#[test]
fn lookup_by_name_blocked_record_is_none() {
    let loaded = registry_with(
        settings(&[], Some(&["203.0.113.0/24"])),
        &[PRIVATE_SERVER_NAME, PUBLIC_SERVER_NAME],
        &[],
    );
    assert!(
        loaded
            .server_by_name("test-shared", Some("8.8.8.8"))
            .is_none()
    );
}

#[test]
fn lookup_filtered_registry_keeps_only_reachable_records() {
    let loaded = registry_with(
        settings(&[], Some(&["203.0.113.0/24"])),
        &[PUBLIC, PRIVATE],
        &[],
    );
    let filtered = loaded.filtered_registry(Some("8.8.8.8"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0, "test-b");
    let all = loaded.filtered_registry(None);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "test-b");
    assert_eq!(all[1].0, "test-a");
}

#[test]
fn expand_ids_stay_as_given() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    assert_eq!(
        loaded.expand_permission_list(&["test-a".to_owned()]),
        vec!["test-a".to_owned()]
    );
}

#[test]
fn expand_alias_reaches_every_match() {
    let loaded = registry(&[ALIAS_ALPHA, ALIAS_TWIN], &[]);
    assert_eq!(
        loaded.expand_permission_list(&["test-alias".to_owned()]),
        vec!["test-a".to_owned(), "test-b".to_owned()]
    );
}

#[test]
fn expand_server_name_and_name_reach_every_match() {
    let loaded = registry(&[NAMED_GAMMA, CALLED_GAMMA], &[]);
    assert_eq!(
        loaded.expand_permission_list(&["test-gamma".to_owned()]),
        vec!["test-c".to_owned(), "test-d".to_owned()]
    );
}

#[test]
fn expand_known_id_wins_over_a_record_name() {
    let loaded = registry(&[ALIAS_ALPHA, NAMED_AFTER_ID], &[]);
    assert_eq!(
        loaded.expand_permission_list(&["test-a".to_owned()]),
        vec!["test-a".to_owned()]
    );
}

#[test]
fn expand_unknown_identifier_is_kept() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    assert_eq!(
        loaded.expand_permission_list(&["test-ghost".to_owned()]),
        vec!["test-ghost".to_owned()]
    );
}

#[test]
fn expand_deduplicates_first_seen() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    assert_eq!(
        loaded.expand_permission_list(&[
            "test-alias".to_owned(),
            "test-a".to_owned(),
            "test-ghost".to_owned(),
            "test-ghost".to_owned(),
        ]),
        vec!["test-a".to_owned(), "test-ghost".to_owned()]
    );
}

#[test]
fn expand_empty_list_is_empty() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    assert!(loaded.expand_permission_list(&[]).is_empty());
}

#[test]
fn expand_tool_permissions_unions_tools_per_server() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    let mut permissions = BTreeMap::new();
    permissions.insert("test-a".to_owned(), vec!["t1".to_owned(), "t2".to_owned()]);
    permissions.insert(
        "test-alias".to_owned(),
        vec!["t2".to_owned(), "t3".to_owned()],
    );
    let expanded = loaded.expand_tool_permissions(Some(&permissions));
    assert_eq!(expanded.len(), 1);
    assert_eq!(
        expanded["test-a"],
        vec!["t1".to_owned(), "t2".to_owned(), "t3".to_owned()]
    );
}

#[test]
fn expand_tool_permissions_groups_sorted_by_id() {
    let loaded = registry(&[ALIAS_ALPHA, ALIAS_TWIN], &[]);
    let mut permissions = BTreeMap::new();
    permissions.insert("test-b".to_owned(), vec!["y".to_owned()]);
    permissions.insert("test-alias".to_owned(), vec!["x".to_owned()]);
    let expanded = loaded.expand_tool_permissions(Some(&permissions));
    let ids: Vec<&str> = expanded.keys().map(String::as_str).collect();
    assert_eq!(ids, vec!["test-a", "test-b"]);
    assert_eq!(expanded["test-a"], vec!["x".to_owned()]);
    assert_eq!(expanded["test-b"], vec!["x".to_owned(), "y".to_owned()]);
}

#[test]
fn expand_tool_permissions_keeps_the_wildcard() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    let mut permissions = BTreeMap::new();
    permissions.insert("test-a".to_owned(), vec!["*".to_owned()]);
    let expanded = loaded.expand_tool_permissions(Some(&permissions));
    assert_eq!(expanded["test-a"], vec!["*".to_owned()]);
}

#[test]
fn expand_tool_permissions_keeps_an_empty_tool_list() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    let mut permissions = BTreeMap::new();
    permissions.insert("test-a".to_owned(), Vec::new());
    let expanded = loaded.expand_tool_permissions(Some(&permissions));
    assert!(expanded.contains_key("test-a"));
    assert!(expanded["test-a"].is_empty());
}

#[test]
fn expand_tool_permissions_without_permissions_is_empty() {
    let loaded = registry(&[ALIAS_ALPHA], &[]);
    assert!(loaded.expand_tool_permissions(None).is_empty());
    assert!(
        loaded
            .expand_tool_permissions(Some(&BTreeMap::new()))
            .is_empty()
    );
}

#[test]
fn table_config_declared_server_is_config() {
    let tables = registry(&[PUBLIC], &[]).all_server_tables();
    assert_eq!(tables[0].is_config, RowFlag(true));
}

#[test]
fn table_config_server_overridden_by_a_row_is_not_config() {
    let loaded = registry(&[CONFIG_BETA], &[DB_BETA]);
    let tables = loaded.all_server_tables();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].url.as_deref(), Some("https://db.example"));
    assert_eq!(tables[0].is_config, RowFlag(false));
}

#[test]
fn table_database_only_server_is_not_config() {
    let tables = registry(&[], &[DB_ONLY]).all_server_tables();
    assert_eq!(tables[0].is_config, RowFlag(false));
}

#[test]
fn table_copies_identity_fields() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.server_id, "test-full");
    assert_eq!(row.server_name.as_deref(), Some("test-full-name"));
    assert_eq!(row.alias.as_deref(), Some("test-full-alias"));
    assert_eq!(row.description.as_deref(), Some("A full server"));
    assert_eq!(row.url.as_deref(), Some("https://mcp.example/mcp"));
    assert_eq!(row.spec_path.as_deref(), Some("/specs/full.json"));
    assert_eq!(row.transport, MCPTransport::Sse);
    assert_eq!(row.auth_type, Some(MCPAuth::Oauth2));
    assert_eq!(row.instructions.as_deref(), Some("Use it"));
    assert!(row.mcp_info.is_some());
}

#[test]
fn table_copies_tool_maps() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.mcp_access_groups, vec!["group-a".to_owned()]);
    assert_eq!(row.allowed_tools, vec!["echo".to_owned(), "sum".to_owned()]);
    assert_eq!(row.extra_headers, vec!["X-Extra".to_owned()]);
    let display_names = row.tool_name_to_display_name.as_ref().unwrap();
    assert_eq!(display_names.get("echo").map(String::as_str), Some("Echo"));
    let descriptions = row.tool_name_to_description.as_ref().unwrap();
    assert_eq!(descriptions.get("echo").map(String::as_str), Some("Echoes"));
    assert!(row.pinned_tools.as_ref().unwrap().contains_key("echo"));
}

#[test]
fn table_copies_oauth_fields() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.issuer.as_deref(), Some("https://issuer.example"));
    assert_eq!(row.oauth2_flow, Some(McpOAuth2Flow::ClientCredentials));
    assert_eq!(
        row.token_exchange_endpoint.as_deref(),
        Some("https://exchange.example/token")
    );
    assert_eq!(row.audience.as_deref(), Some("aud"));
    assert_eq!(row.subject_token_type.as_deref(), Some("urn:custom"));
    assert_eq!(
        row.token_exchange_profile.as_deref(),
        Some("custom-profile")
    );
}

#[test]
fn table_copies_flags() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.allow_all_keys, RowFlag(true));
    assert_eq!(row.available_on_public_internet, PublicInternet(false));
    assert_eq!(row.delegate_auth_to_upstream, RowFlag(true));
    assert_eq!(row.oauth_passthrough, RowFlag(true));
    assert_eq!(row.dcr_bridge, Some(true));
    assert_eq!(row.per_server_oauth_discovery, RowFlag(true));
    assert_eq!(row.is_byok, RowFlag(true));
    assert_eq!(row.byok_description, vec!["bring your own".to_owned()]);
    assert_eq!(
        row.byok_api_key_help_url.as_deref(),
        Some("https://help.example")
    );
    assert_eq!(row.source_url.as_deref(), Some("https://source.example"));
    assert_eq!(row.command.as_deref(), Some("run-server"));
    assert_eq!(row.args, vec!["--flag".to_owned()]);
}

#[test]
fn table_copies_limits() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.timeout, Some(Decimal::try_from(2.5_f64).unwrap()));
    assert_eq!(row.max_concurrent_requests, Some(7));
}

#[test]
fn table_credentials_keep_the_configured_scopes() {
    let tables = registry(&[SCOPED], &[]).all_server_tables();
    let credentials = tables[0].credentials.as_ref().unwrap();
    assert_eq!(
        credentials.scopes,
        Some(Some(vec!["read".to_owned(), "write".to_owned()]))
    );
}

#[test]
fn table_empty_configured_scopes_give_no_credentials() {
    let tables = registry(&[EMPTY_SCOPES], &[]).all_server_tables();
    assert!(tables[0].credentials.is_none());
}

#[test]
fn table_absent_configured_scopes_give_no_credentials() {
    let tables = registry(&[PUBLIC], &[]).all_server_tables();
    assert!(tables[0].credentials.is_none());
}

#[test]
fn table_description_comes_from_mcp_info() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    assert_eq!(tables[0].description.as_deref(), Some("A full server"));
}

#[test]
fn table_missing_description_is_none() {
    let tables = registry(&[PUBLIC], &[]).all_server_tables();
    assert_eq!(tables[0].description, None);
}

#[test]
fn table_non_string_description_is_dropped() {
    let tables = registry(&[NUMERIC_DESCRIPTION], &[]).all_server_tables();
    assert_eq!(tables[0].description, None);
}

#[test]
fn table_configured_urls_win_over_the_live_urls() {
    let tables = registry(&[CONFIGURED_URLS], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(
        row.authorization_url.as_deref(),
        Some("https://configured/auth")
    );
    assert_eq!(row.token_url.as_deref(), Some("https://configured/token"));
    assert_eq!(
        row.registration_url.as_deref(),
        Some("https://configured/register")
    );
}

#[test]
fn table_live_urls_are_used_without_configured_ones() {
    let tables = registry(&[LIVE_URLS], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.authorization_url.as_deref(), Some("https://live/auth"));
    assert_eq!(row.token_url.as_deref(), Some("https://live/token"));
    assert_eq!(
        row.registration_url.as_deref(),
        Some("https://live/register")
    );
}

#[test]
fn table_explicitly_empty_configured_url_is_kept() {
    let tables = registry(&[EMPTY_CONFIGURED_URL], &[]).all_server_tables();
    assert_eq!(tables[0].authorization_url.as_deref(), Some(""));
}

#[test]
fn table_empty_lists_and_maps_default() {
    let tables = registry(&[PUBLIC], &[]).all_server_tables();
    let row = &tables[0];
    assert!(row.teams.is_empty());
    assert!(row.mcp_access_groups.is_empty());
    assert!(row.allowed_tools.is_empty());
    assert!(row.extra_headers.is_empty());
    assert!(row.args.is_empty());
    assert!(row.env.0.is_empty());
    assert!(row.env_vars.is_none());
    assert!(row.static_headers.is_none());
    assert_eq!(
        row.subject_token_type.as_deref(),
        Some(DEFAULT_SUBJECT_TOKEN_TYPE)
    );
    assert_eq!(row.token_exchange_profile.as_deref(), Some("rfc8693"));
    assert_eq!(row.available_on_public_internet, PublicInternet(true));
    assert_eq!(row.allow_all_keys, RowFlag(false));
}

#[test]
fn table_env_vars_become_models() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let env_vars = tables[0].env_vars.as_ref().unwrap();
    assert_eq!(env_vars.len(), 1);
    assert_eq!(env_vars[0].name, "EV");
    assert_eq!(env_vars[0].value, "env-value");
    assert_eq!(env_vars[0].scope, McpEnvVarScope::Global);
    assert_eq!(env_vars[0].description, None);
}

#[test]
fn table_invalid_env_var_is_dropped() {
    let tables = registry(&[BAD_ENV_VAR], &[]).all_server_tables();
    let env_vars = tables[0].env_vars.as_ref().unwrap();
    assert_eq!(env_vars.len(), 1);
    assert_eq!(env_vars[0].name, "GOOD");
}

#[test]
fn table_static_headers_and_env_stay_secret() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    let static_headers = row.static_headers.as_ref().unwrap();
    assert_eq!(
        static_headers.0["X-Static"].expose_secret(),
        "static-secret"
    );
    assert_eq!(row.env.0["ENVKEY"].expose_secret(), "env-secret");
}

#[test]
fn table_health_fields_are_unset() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    assert_eq!(row.status, None);
    assert_eq!(row.last_health_check, None);
    assert_eq!(row.health_check_error, None);
    assert_eq!(row.has_user_credential, None);
    assert_eq!(row.connected_app_reachable, None);
    assert_eq!(row.created_by, None);
    assert_eq!(row.updated_by, None);
}

#[test]
fn table_approval_status_is_active() {
    let tables = registry(&[PUBLIC], &[]).all_server_tables();
    assert_eq!(tables[0].approval_status.as_deref(), Some("active"));
}

#[test]
fn table_timestamps_keep_the_instant() {
    let tables = registry(&[FULL_RECORD], &[]).all_server_tables();
    let row = &tables[0];
    let created = serde_json::to_string(&row.created_at.unwrap()).unwrap();
    assert_eq!(created, "\"2026-01-02T03:04:05Z\"");
    let updated = serde_json::to_string(&row.updated_at.unwrap()).unwrap();
    assert_eq!(updated, "\"2026-02-03T04:05:06.500000Z\"");
}

#[test]
fn table_all_server_tables_follow_registry_order() {
    let loaded = registry(&[PUBLIC, PRIVATE], &[DB_ONLY]);
    let tables = loaded.all_server_tables();
    let ids: Vec<&str> = tables
        .iter()
        .map(|table| table.server_id.as_str())
        .collect();
    assert_eq!(ids, vec!["test-b", "test-a", "test-z"]);
}

#[test]
fn table_empty_registry_has_no_tables() {
    assert!(registry(&[], &[]).all_server_tables().is_empty());
}
