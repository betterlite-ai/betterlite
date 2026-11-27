use crate::registry::servers::ServerRegistry;

impl ServerRegistry {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B4718, B4720, B4721 and B4722 read the live list through this from betterlite-server
    pub fn public_mcp_servers(&self) -> &[String] {
        &self.settings.public_mcp_servers
    }

    // ast-grep-ignore: no-one-line-functions - B8627 replaces the live list through this from the make-public route
    pub fn set_public_mcp_servers(&mut self, ids: Vec<String>) {
        self.settings.public_mcp_servers = ids;
    }
}

mod tests;
