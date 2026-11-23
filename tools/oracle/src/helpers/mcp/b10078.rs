use betterlite_mcp::access_group_servers::config_server_ids_for_access_groups;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    servers: Vec<MCPServer>,
    access_groups: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = ServerRegistry::new(
            RegistrySettings {
                public_mcp_servers: Vec::new(),
                public_hub_strict: true,
                internal_ip_ranges: None,
            },
            input.servers.clone(),
            Vec::new(),
        );
        let mut server_ids = config_server_ids_for_access_groups(&registry, &input.access_groups);
        server_ids.sort();
        Ok(json!(server_ids))
    }
}
