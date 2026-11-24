use crate::registry::servers::ServerRegistry;
use crate::server_record::MCPServer;

pub trait PublicServers {
    #[must_use]
    fn is_server_public(&self, server_id: &str, public_ids: Option<&[String]>) -> bool;

    #[must_use]
    fn public_servers(&self) -> Vec<&MCPServer>;
}

impl PublicServers for ServerRegistry {
    fn is_server_public(&self, server_id: &str, public_ids: Option<&[String]>) -> bool {
        let Some(record) = self
            .db_servers
            .get(server_id)
            .or_else(|| self.config_servers.get(server_id))
        else {
            return false;
        };
        let published_ids = public_ids.unwrap_or(self.settings.public_mcp_servers.as_slice());
        published_ids
            .iter()
            .any(|published_id| published_id.as_str() == server_id)
            || (!self.settings.public_hub_strict && record.fields.available_on_public_internet)
    }

    fn public_servers(&self) -> Vec<&MCPServer> {
        let public_ids = self.settings.public_mcp_servers.as_slice();
        self.get_registry()
            .into_iter()
            .filter(|server| self.is_server_public(&server.fields.server_id, Some(public_ids)))
            .collect()
    }
}

mod tests;
