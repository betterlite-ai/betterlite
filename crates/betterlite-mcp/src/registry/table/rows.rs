use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::{
    McpEnvVar, McpOAuth2Flow, McpServerTable, default_approval_status,
};
use betterlite_db::managed_resource_rows::wire::{
    PublicInternet, RowFlag, RowTimestamp, SecretMap, SecretMapField,
};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::MCPCredentials;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tracing::warn;

use crate::server_record::{OAuth2Flow, ServerFields};

pub(super) fn fill_identity(row: &mut McpServerTable, fields: &ServerFields) {
    row.server_id.clone_from(&fields.server_id);
    row.server_name.clone_from(&fields.server_name);
    row.alias.clone_from(&fields.alias);
    row.description = mcp_info_description(fields.mcp_info.as_ref(), &fields.server_id);
    row.url.clone_from(&fields.url);
    row.spec_path.clone_from(&fields.spec_path);
    row.transport = fields.transport;
    row.auth_type = fields.auth_type;
    row.created_at = row_timestamp(fields.created_at, &fields.server_id);
    row.updated_at = row_timestamp(fields.updated_at, &fields.server_id);
    row.mcp_info.clone_from(&fields.mcp_info);
    row.instructions.clone_from(&fields.instructions);
}

pub(super) fn fill_tools(row: &mut McpServerTable, fields: &ServerFields) {
    row.mcp_access_groups = fields.access_groups.clone().unwrap_or_default();
    row.allowed_tools = fields.allowed_tools.clone().unwrap_or_default();
    row.tool_name_to_display_name
        .clone_from(&fields.tool_name_to_display_name);
    row.tool_name_to_description
        .clone_from(&fields.tool_name_to_description);
    row.pinned_tools.clone_from(&fields.pinned_tools);
    row.extra_headers = fields.extra_headers.clone().unwrap_or_default();
    row.env_vars = fields
        .env_vars
        .as_deref()
        .map(|entries| env_var_models(entries, &fields.server_id));
}

pub(super) fn fill_auth(row: &mut McpServerTable, fields: &ServerFields) {
    row.credentials = fields
        .configured_scopes
        .as_deref()
        .filter(|scopes| !scopes.is_empty())
        .map(credentials);
    row.static_headers = fields
        .static_headers
        .as_ref()
        .map(|values| SecretMapField(secret_map(values)));
    row.env = SecretMapField(fields.env.as_ref().map_or_else(SecretMap::new, secret_map));
    row.issuer.clone_from(&fields.issuer);
    row.authorization_url = fields
        .configured_authorization_url
        .clone()
        .or_else(|| fields.authorization_url.clone());
    row.token_url = fields
        .configured_token_url
        .clone()
        .or_else(|| fields.token_url.clone());
    row.registration_url = fields
        .configured_registration_url
        .clone()
        .or_else(|| fields.registration_url.clone());
}

pub(super) fn fill_oauth(row: &mut McpServerTable, fields: &ServerFields) {
    row.oauth2_flow = fields.oauth2_flow.map(row_oauth2_flow);
    row.token_exchange_endpoint
        .clone_from(&fields.token_exchange_endpoint);
    row.audience.clone_from(&fields.audience);
    row.subject_token_type = Some(fields.subject_token_type.clone());
    row.token_exchange_profile = Some(fields.token_exchange_profile.clone());
}

pub(super) fn fill_flags(row: &mut McpServerTable, fields: &ServerFields) {
    row.command.clone_from(&fields.command);
    row.args = fields.args.clone().unwrap_or_default();
    row.allow_all_keys = RowFlag(fields.allow_all_keys);
    row.available_on_public_internet = PublicInternet(fields.available_on_public_internet);
    row.delegate_auth_to_upstream = RowFlag(fields.delegate_auth_to_upstream);
    row.oauth_passthrough = RowFlag(fields.oauth_passthrough);
    row.dcr_bridge = fields.dcr_bridge;
    row.per_server_oauth_discovery = RowFlag(fields.per_server_oauth_discovery);
    row.is_byok = RowFlag(fields.is_byok);
    row.byok_description.clone_from(&fields.byok_description);
    row.byok_api_key_help_url
        .clone_from(&fields.byok_api_key_help_url);
    row.source_url.clone_from(&fields.source_url);
    row.approval_status = default_approval_status();
}

pub(super) fn fill_limits(row: &mut McpServerTable, fields: &ServerFields) {
    row.timeout = timeout_decimal(fields.timeout, &fields.server_id);
    row.max_concurrent_requests = fields.max_concurrent_requests;
}

// LiteLLM's _env_vars_to_models: one MCPEnvVar per entry, with a bad entry logged and dropped
fn env_var_models(entries: &[BTreeMap<String, JsonValue>], server_id: &str) -> Vec<McpEnvVar> {
    let mut models = Vec::with_capacity(entries.len());
    for entry in entries {
        let Ok(encoded) = serde_json::to_string(entry) else {
            warn!("MCP server {server_id}: an env var entry cannot be encoded");
            continue;
        };
        match serde_json::from_str::<McpEnvVar>(&encoded) {
            Ok(model) => models.push(model),
            Err(error) => {
                warn!("MCP server {server_id}: an env var entry is not a valid MCPEnvVar: {error}");
            }
        }
    }
    models
}

fn credentials(scopes: &[String]) -> MCPCredentials {
    MCPCredentials {
        auth_value: None,
        client_id: None,
        client_secret: None,
        scopes: Some(Some(scopes.to_vec())),
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_region_name: None,
        aws_service_name: None,
        aws_role_name: None,
        aws_session_name: None,
        audience: None,
        token_exchange_endpoint: None,
        subject_token_type: None,
        id_jag_resource_token_endpoint: None,
        id_jag_resource: None,
        upstream_resource: None,
        upstream_token_header: None,
        client_private_key: None,
        client_private_key_id: None,
        client_assertion_signing_alg: None,
        token_endpoint_auth_method: None,
        redirect_uris: None,
        token_exchange_profile: None,
    }
}

fn secret_map(values: &BTreeMap<String, String>) -> SecretMap {
    let mut secrets = SecretMap::new();
    for (name, value) in values {
        secrets.insert(name.clone(), SecretString::from(value.clone()));
    }
    secrets
}

// pydantic stores a datetime through the row's own text form, which only its deserializer accepts
fn row_timestamp(value: Option<DateTime<Utc>>, server_id: &str) -> Option<RowTimestamp> {
    let timestamp = value?;
    let encoded = serde_json::to_string(&timestamp)
        .inspect_err(|error| {
            warn!("MCP server {server_id}: a timestamp cannot be encoded: {error}")
        })
        .ok()?;
    serde_json::from_str(&encoded)
        .inspect_err(|error| warn!("MCP server {server_id}: a timestamp cannot be stored: {error}"))
        .ok()
}

// LiteLLM reads mcp_info["description"]; a value of another type is logged and left out
fn mcp_info_description(info: Option<&MCPInfo>, server_id: &str) -> Option<String> {
    let encoded = serde_json::to_string(info?)
        .inspect_err(|error| warn!("MCP server {server_id}: mcp_info cannot be encoded: {error}"))
        .ok()?;
    let metadata: BTreeMap<String, JsonValue> = serde_json::from_str(&encoded)
        .inspect_err(|error| warn!("MCP server {server_id}: mcp_info cannot be decoded: {error}"))
        .ok()?;
    match metadata.get("description") {
        Some(JsonValue::String(description)) => Some(description.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => {
            warn!("MCP server {server_id}: mcp_info description is not a string");
            None
        }
        None => None,
    }
}

fn row_oauth2_flow(flow: OAuth2Flow) -> McpOAuth2Flow {
    match flow {
        OAuth2Flow::ClientCredentials => McpOAuth2Flow::ClientCredentials,
        OAuth2Flow::AuthorizationCode => McpOAuth2Flow::AuthorizationCode,
    }
}

// the table stores a float as a decimal; a value no decimal holds is logged and left out
fn timeout_decimal(value: Option<f64>, server_id: &str) -> Option<Decimal> {
    let seconds = value?;
    match Decimal::try_from(seconds) {
        Ok(decimal) => Some(decimal),
        Err(error) => {
            warn!("MCP server {server_id}: timeout {seconds} has no decimal form: {error}");
            None
        }
    }
}
