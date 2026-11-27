use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::MCPCredentials;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::admin_credential_view::redact_mcp_credentials;
use serde::{Deserialize, Serialize};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    server: McpServerTable,
    config_servers: Vec<MCPServer>,
    db_servers: Vec<MCPServer>,
    #[serde(default)]
    public_mcp_servers: Option<Vec<String>>,
    public_hub_strict: bool,
}

#[derive(Debug, Serialize)]
struct Answer {
    credentials: Option<MCPCredentials>,
    mcp_info: Option<MCPInfo>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let public_ids = input.public_mcp_servers.clone().unwrap_or_default();
        let registry = ServerRegistry::new(
            RegistrySettings {
                public_mcp_servers: public_ids.clone(),
                public_hub_strict: input.public_hub_strict,
                internal_ip_ranges: None,
            },
            input.config_servers.clone(),
            input.db_servers.clone(),
        );
        let view =
            redact_mcp_credentials(&input.server, &registry, &public_ids).or_config_error()?;
        serde_json::to_value(Answer {
            credentials: view.credentials,
            mcp_info: view.mcp_info,
        })
        .or_config_error()
    }
}
