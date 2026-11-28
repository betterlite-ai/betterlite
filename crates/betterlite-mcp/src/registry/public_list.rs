use crate::registry::servers::ServerRegistry;

pub trait PublicMcpServers {
    #[must_use]
    fn public_mcp_servers(&self) -> &[String];

    fn set_public_mcp_servers(&mut self, ids: Vec<String>);
}

impl PublicMcpServers for ServerRegistry {
    fn public_mcp_servers(&self) -> &[String] {
        &self.settings.public_mcp_servers
    }

    fn set_public_mcp_servers(&mut self, ids: Vec<String>) {
        self.settings.public_mcp_servers = ids;
    }
}

mod tests;
