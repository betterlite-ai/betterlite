use betterlite_mcp::registry::public_servers::PublicServers as _;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    List {
        config_servers: Vec<MCPServer>,
        db_servers: Vec<MCPServer>,
        #[serde(default)]
        public_mcp_servers: Option<Vec<String>>,
        public_hub_strict: bool,
    },
    IsPublic {
        config_servers: Vec<MCPServer>,
        db_servers: Vec<MCPServer>,
        #[serde(default)]
        public_mcp_servers: Option<Vec<String>>,
        public_hub_strict: bool,
        server_id: String,
        #[serde(default)]
        public_ids: Option<Vec<String>>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::List {
                config_servers,
                db_servers,
                public_mcp_servers,
                public_hub_strict,
            } => {
                let registry = build_registry(
                    config_servers,
                    db_servers,
                    public_mcp_servers.as_deref(),
                    *public_hub_strict,
                );
                let server_ids: Vec<&str> = registry
                    .public_servers()
                    .into_iter()
                    .map(|server| server.fields.server_id.as_str())
                    .collect();
                Ok(json!(server_ids))
            }
            Input::IsPublic {
                config_servers,
                db_servers,
                public_mcp_servers,
                public_hub_strict,
                server_id,
                public_ids,
            } => {
                let registry = build_registry(
                    config_servers,
                    db_servers,
                    public_mcp_servers.as_deref(),
                    *public_hub_strict,
                );
                let server_is_public = registry.is_server_public(server_id, public_ids.as_deref());
                Ok(json!(server_is_public))
            }
        }
    }
}

fn build_registry(
    config_servers: &[MCPServer],
    db_servers: &[MCPServer],
    public_mcp_servers: Option<&[String]>,
    public_hub_strict: bool,
) -> ServerRegistry {
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: public_mcp_servers.unwrap_or_default().to_vec(),
            public_hub_strict,
            internal_ip_ranges: None,
        },
        config_servers.to_vec(),
        db_servers.to_vec(),
    )
}
