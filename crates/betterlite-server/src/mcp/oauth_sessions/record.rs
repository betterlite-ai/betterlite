use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::{
    McpServerTable, default_approval_status, default_status,
};
use betterlite_db::managed_resource_rows::wire::{
    PublicInternet, RowFlag, RowTimestamp, SecretMap, SecretMapField,
};
use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_mcp::server_payload::request::{NewMCPServerRequest, OrderedMap};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::SecretString;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("the timeout {value} cannot be stored as a decimal")]
    Timeout {
        value: f64,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the timestamp {value} cannot be stored as a row timestamp")]
    Timestamp {
        value: String,
        #[source]
        source: serde_json::Error,
    },
}

// Every field at its empty value; the fills below copy the request over the ones a session sets
const EMPTY_ROW: McpServerTable = McpServerTable {
    server_id: String::new(),
    is_config: RowFlag(false),
    server_name: None,
    alias: None,
    description: None,
    url: None,
    spec_path: None,
    transport: MCPTransport::Http,
    auth_type: None,
    credentials: None,
    instructions: None,
    created_at: None,
    created_by: None,
    updated_at: None,
    updated_by: None,
    teams: Vec::new(),
    mcp_access_groups: Vec::new(),
    allowed_tools: Vec::new(),
    tool_name_to_display_name: None,
    tool_name_to_description: None,
    pinned_tools: None,
    extra_headers: Vec::new(),
    mcp_info: None,
    static_headers: None,
    env_vars: None,
    status: None,
    last_health_check: None,
    health_check_error: None,
    command: None,
    args: Vec::new(),
    env: SecretMapField(SecretMap::new()),
    issuer: None,
    authorization_url: None,
    token_url: None,
    registration_url: None,
    oauth2_flow: None,
    token_exchange_endpoint: None,
    audience: None,
    subject_token_type: None,
    token_exchange_profile: None,
    allow_all_keys: RowFlag(false),
    available_on_public_internet: PublicInternet(false),
    delegate_auth_to_upstream: RowFlag(false),
    oauth_passthrough: RowFlag(false),
    dcr_bridge: None,
    per_server_oauth_discovery: RowFlag(false),
    is_byok: RowFlag(false),
    byok_description: Vec::new(),
    byok_api_key_help_url: None,
    has_user_credential: None,
    connected_app_reachable: None,
    source_url: None,
    timeout: None,
    max_concurrent_requests: None,
    approval_status: None,
    submitted_by: None,
    submitted_at: None,
    reviewed_at: None,
    review_notes: None,
};

// LiteLLM's _build_temporary_mcp_server_record: the row a session runs on before anything is saved
pub fn build_temporary_record(
    request: &NewMCPServerRequest,
    created_by: Option<&str>,
    server_id: &str,
    now: DateTime<Utc>,
) -> Result<McpServerTable, RecordError> {
    let mut row = EMPTY_ROW;
    fill_request(&mut row, request, server_id);
    fill_audit(&mut row, request, created_by, now)?;
    Ok(row)
}

// every request field the record keeps, with the name fallback LiteLLM gives the server
fn fill_request(row: &mut McpServerTable, request: &NewMCPServerRequest, server_id: &str) {
    row.server_id = server_id.to_owned();
    row.server_name = Some(temporary_server_name(request, server_id));
    row.alias = request.alias.clone();
    row.description = request.description.clone();
    row.url = request.url.clone();
    row.transport = request.transport;
    row.auth_type = request.auth_type;
    row.credentials = request.credentials.clone();
    row.mcp_access_groups = request.mcp_access_groups.clone();
    row.allowed_tools = request.allowed_tools.clone().unwrap_or_default();
    row.extra_headers = request.extra_headers.clone().unwrap_or_default();
    row.mcp_info = request.mcp_info.clone();
    row.static_headers = request
        .static_headers
        .as_ref()
        .map(|headers| SecretMapField(secret_values(headers)));
    row.command = request.command.clone();
    row.args = request.args.clone();
    row.env = SecretMapField(secret_values(&request.env));
    row.issuer = request.issuer.clone();
    row.authorization_url = request.authorization_url.clone();
    row.token_url = request.token_url.clone();
    row.registration_url = request.registration_url.clone();
}

// the timestamps and defaults LiteLLM stamps on a row it has not saved yet
fn fill_audit(
    row: &mut McpServerTable,
    request: &NewMCPServerRequest,
    created_by: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), RecordError> {
    let timestamp = stored_timestamp(now)?;
    let audit = created_by.map(str::to_owned);
    row.created_at = Some(timestamp);
    row.created_by = audit.clone();
    row.updated_at = Some(timestamp);
    row.updated_by = audit;
    row.status = default_status();
    row.allow_all_keys = RowFlag(request.allow_all_keys);
    row.available_on_public_internet = PublicInternet(request.available_on_public_internet);
    row.timeout = timeout_decimal(request.timeout)?;
    row.max_concurrent_requests = request.max_concurrent_requests;
    row.approval_status = default_approval_status();
    Ok(())
}

// LiteLLM's `payload.server_name or payload.alias or server_id`, an empty string reading as absent
fn temporary_server_name(request: &NewMCPServerRequest, server_id: &str) -> String {
    request
        .server_name
        .clone()
        .filter(|name| !name.is_empty())
        .or_else(|| request.alias.clone().filter(|alias| !alias.is_empty()))
        .unwrap_or_else(|| server_id.to_owned())
}

fn secret_values(headers: &OrderedMap) -> BTreeMap<String, SecretString> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

fn timeout_decimal(timeout: Option<f64>) -> Result<Option<Decimal>, RecordError> {
    let Some(value) = timeout else {
        return Ok(None);
    };
    Decimal::try_from(value)
        .map(Some)
        .map_err(|source| RecordError::Timeout { value, source })
}

// the row's timestamp has no public constructor, so its own wire form is how a value enters it
fn stored_timestamp(now: DateTime<Utc>) -> Result<RowTimestamp, RecordError> {
    let encoded = serde_json::to_string(&now).map_err(|source| RecordError::Timestamp {
        value: now.to_rfc3339(),
        source,
    })?;
    serde_json::from_str(&encoded).map_err(|source| RecordError::Timestamp {
        value: encoded,
        source,
    })
}
