use betterlite_db::managed_resource_rows::mcp::{
    McpServerTable, McpStatus, default_approval_status,
};
use betterlite_db::managed_resource_rows::wire::{
    PublicInternet, RowFlag, RowTimestamp, SecretMap, SecretMapField,
};
use betterlite_db::mcp_stored_contracts::MCPTransport;
use chrono::{DateTime, SecondsFormat, Utc};

use super::health::Health;
use crate::registry::servers::ServerRegistry;
use crate::registry::table::ServerTables as _;
use crate::server_record::MCPServer;

// The row a health check of an unknown server returns: LiteLLM_MCPServerTable with its declared defaults
const NOT_FOUND_BASE: McpServerTable = McpServerTable {
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
    available_on_public_internet: PublicInternet(true),
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

#[must_use]
pub fn not_found_row(server_id: &str, checked_at: DateTime<Utc>) -> McpServerTable {
    let mut row = NOT_FOUND_BASE;
    row.server_id = server_id.to_owned();
    row.status = Some(McpStatus::Unknown);
    row.approval_status = default_approval_status();
    row.last_health_check = health_timestamp(checked_at);
    row.health_check_error = Some("Server not found".to_owned());
    row
}

// health_check_server builds its row by hand, so every field it leaves out takes the pydantic default
#[must_use]
pub fn session_row(
    registry: &ServerRegistry,
    server: &MCPServer,
    health: &Health,
) -> McpServerTable {
    let mut row = registry.server_table(server);
    row.is_config = RowFlag(false);
    row.spec_path = None;
    row.credentials = None;
    row.tool_name_to_display_name = None;
    row.tool_name_to_description = None;
    row.pinned_tools = None;
    row.available_on_public_internet = PublicInternet(true);
    row.delegate_auth_to_upstream = RowFlag(false);
    row.oauth_passthrough = RowFlag(false);
    row.is_byok = RowFlag(false);
    row.byok_description = Vec::new();
    row.byok_api_key_help_url = None;
    row.source_url = None;
    apply_health(&mut row, health);
    row
}

// the spec_path branch is _build_mcp_server_table with status, error and time replaced
#[must_use]
pub fn spec_row(registry: &ServerRegistry, server: &MCPServer, health: &Health) -> McpServerTable {
    let mut row = registry.server_table(server);
    apply_health(&mut row, health);
    row
}

fn apply_health(row: &mut McpServerTable, health: &Health) {
    row.status = Some(health.status);
    row.health_check_error.clone_from(&health.error);
    row.last_health_check = health_timestamp(health.checked_at);
}

// pydantic carries a datetime as aware ISO text with six fractional digits and Z, which this row reads back
fn health_timestamp(checked_at: DateTime<Utc>) -> Option<RowTimestamp> {
    let text = checked_at.to_rfc3339_opts(SecondsFormat::Micros, true);
    let encoded = serde_json::to_string(&text).ok()?;
    serde_json::from_str(&encoded).ok()?
}
