use betterlite_auth::trusted_network::internal::{is_internal_ip, parse_internal_networks};

use super::servers::ServerRegistry;
use crate::server_record::MCPServer;

impl ServerRegistry {
    // LiteLLM's _is_server_accessible_from_ip
    #[must_use]
    pub fn is_accessible_from_ip(&self, server: &MCPServer, client_ip: Option<&str>) -> bool {
        let Some(ip) = client_ip else {
            return true;
        };
        if server.fields.available_on_public_internet {
            return true;
        }
        let published = self
            .settings
            .public_mcp_servers
            .iter()
            .any(|public_id| public_id.as_str() == server.fields.server_id.as_str());
        if published {
            return true;
        }
        let networks = parse_internal_networks(self.settings.internal_ip_ranges.as_deref());
        is_internal_ip(Some(ip), Some(&networks))
    }

    // LiteLLM's get_mcp_server_by_id
    pub fn server_by_id(&self, server_id: &str, client_ip: Option<&str>) -> Option<&MCPServer> {
        let server = self
            .get_registry()
            .into_iter()
            .find(|server| server.fields.server_id == server_id)?;
        accessible(self, server, client_ip)
    }

    // LiteLLM's get_mcp_server_by_name, whose three passes stop at the first record they match
    pub fn server_by_name(&self, server_name: &str, client_ip: Option<&str>) -> Option<&MCPServer> {
        let registry = self.get_registry();
        let by_alias = registry
            .iter()
            .find(|server| server.fields.alias.as_deref() == Some(server_name))
            .copied();
        if let Some(server) = by_alias {
            return accessible(self, server, client_ip);
        }
        let by_server_name = registry
            .iter()
            .find(|server| server.fields.server_name.as_deref() == Some(server_name))
            .copied();
        if let Some(server) = by_server_name {
            return accessible(self, server, client_ip);
        }
        let by_name = registry
            .iter()
            .find(|server| server.fields.name == server_name)
            .copied();
        if let Some(server) = by_name {
            return accessible(self, server, client_ip);
        }
        None
    }

    // LiteLLM's get_filtered_registry: the server_id and record of every reachable server
    #[must_use]
    pub fn filtered_registry(&self, client_ip: Option<&str>) -> Vec<(String, &MCPServer)> {
        self.get_registry()
            .into_iter()
            .filter(|server| self.is_accessible_from_ip(server, client_ip))
            .map(|server| (server.fields.server_id.clone(), server))
            .collect()
    }
}

// LiteLLM's next(..., None) followed by the ip gate, which a blocked hit does not fall through
fn accessible<'a>(
    registry: &ServerRegistry,
    server: &'a MCPServer,
    client_ip: Option<&str>,
) -> Option<&'a MCPServer> {
    let reachable = registry.is_accessible_from_ip(server, client_ip);
    reachable.then_some(server)
}
