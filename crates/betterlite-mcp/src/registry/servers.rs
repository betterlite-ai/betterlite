use std::collections::BTreeSet;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use indexmap::IndexMap;
use tokio::sync::RwLock;

use crate::server_record::MCPServer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrySettings {
    pub public_mcp_servers: Vec<String>,
    pub public_hub_strict: bool,
    pub internal_ip_ranges: Option<Vec<String>>,
}

pub struct ServerRegistry {
    pub(crate) config_servers: IndexMap<String, MCPServer>,
    pub(crate) db_servers: IndexMap<String, MCPServer>,
    pub(crate) settings: RegistrySettings,
}

pub type SharedRegistry = Arc<RwLock<ServerRegistry>>;

impl ServerRegistry {
    #[must_use]
    pub fn new(
        settings: RegistrySettings,
        config_servers: Vec<MCPServer>,
        db_servers: Vec<MCPServer>,
    ) -> Self {
        Self {
            config_servers: index_by_server_id(config_servers),
            db_servers: index_by_server_id(db_servers),
            settings,
        }
    }

    #[must_use]
    pub fn get_registry(&self) -> Vec<&MCPServer> {
        let mut registry = Vec::new();
        for (server_id, config_server) in &self.config_servers {
            registry.push(self.db_servers.get(server_id).unwrap_or(config_server));
        }
        for (server_id, db_server) in &self.db_servers {
            if !self.config_servers.contains_key(server_id) {
                registry.push(db_server);
            }
        }
        registry
    }

    #[must_use]
    pub fn get_all_mcp_server_ids(&self) -> BTreeSet<String> {
        self.get_registry()
            .into_iter()
            .map(|server| server.fields.server_id.clone())
            .collect()
    }

    #[must_use]
    pub fn get_allow_all_keys_server_ids(&self) -> Vec<String> {
        self.get_registry()
            .into_iter()
            .filter(|server| server.fields.allow_all_keys)
            .map(|server| server.fields.server_id.clone())
            .collect()
    }

    #[must_use]
    pub fn is_config_declared_server(&self, server_id: &str) -> bool {
        // contains_key is the O(1) call, but a one-line body is banned and this read stays a method
        self.config_servers
            .keys()
            .any(|declared_id| declared_id == server_id)
    }

    #[must_use]
    pub fn get_mcp_servers_from_ids(&self, server_ids: &[String]) -> Vec<&MCPServer> {
        self.get_registry()
            .into_iter()
            .filter(|server| server_ids.contains(&server.fields.server_id))
            .collect()
    }
}

impl Debug for ServerRegistry {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerRegistry")
            .field("config_servers", &self.config_servers)
            .field("db_servers", &self.db_servers)
            .field("settings", &self.settings)
            .finish()
    }
}

fn index_by_server_id(servers: Vec<MCPServer>) -> IndexMap<String, MCPServer> {
    let mut by_server_id = IndexMap::new();
    for server in servers {
        by_server_id.insert(server.fields.server_id.clone(), server);
    }
    by_server_id
}

mod tests;
