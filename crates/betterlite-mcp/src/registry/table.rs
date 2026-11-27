use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::managed_resource_rows::wire::{
    PublicInternet, RowFlag, SecretMap, SecretMapField,
};
use betterlite_db::mcp_stored_contracts::MCPTransport;

use self::auth::{fill_auth, fill_oauth};
use self::rows::{fill_flags, fill_identity, fill_limits, fill_tools};
use super::servers::ServerRegistry;
use crate::server_record::MCPServer;

mod auth;
mod rows;

// Every field of the row at its empty value; the fills below copy the record over the ones it sets
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

pub trait ServerTables {
    #[must_use]
    fn server_table(&self, server: &MCPServer) -> McpServerTable;

    #[must_use]
    fn all_server_tables(&self) -> Vec<McpServerTable>;
}

impl ServerTables for ServerRegistry {
    // LiteLLM's _build_mcp_server_table
    fn server_table(&self, server: &MCPServer) -> McpServerTable {
        let fields = &server.fields;
        let mut row = EMPTY_ROW;
        let declared = self.is_config_declared_server(&fields.server_id);
        row.is_config = RowFlag(declared && !self.db_servers.contains_key(&fields.server_id));
        fill_identity(&mut row, fields);
        fill_tools(&mut row, fields);
        fill_auth(&mut row, fields);
        fill_oauth(&mut row, fields);
        fill_flags(&mut row, fields);
        fill_limits(&mut row, fields);
        row
    }

    // LiteLLM's get_all_mcp_servers_unfiltered
    fn all_server_tables(&self) -> Vec<McpServerTable> {
        self.get_registry()
            .into_iter()
            .map(|server| self.server_table(server))
            .collect()
    }
}
