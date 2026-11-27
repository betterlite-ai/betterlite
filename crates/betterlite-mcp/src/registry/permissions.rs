use std::collections::BTreeMap;
use std::slice::from_ref;

use indexmap::IndexSet;
use tracing::debug;

use super::servers::ServerRegistry;
use crate::server_record::MCPServer;

impl ServerRegistry {
    // LiteLLM's expand_permission_list: ids, then aliases and names, and anything else as given
    #[must_use]
    pub fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String> {
        let registry = self.get_registry();
        let mut expanded: IndexSet<String> = IndexSet::new();
        for identifier in identifiers {
            let known = registry
                .iter()
                .any(|server| server.fields.server_id.as_str() == identifier.as_str());
            if known {
                expanded.insert(identifier.clone());
                continue;
            }
            let mut matched = false;
            for server in &registry {
                if names_server(server, identifier) {
                    expanded.insert(server.fields.server_id.clone());
                    matched = true;
                }
            }
            if !matched {
                debug!(
                    "MCP permission entry {identifier:?} is not a known server; keeping it for the access check"
                );
                expanded.insert(identifier.clone());
            }
        }
        expanded.into_iter().collect()
    }

    // LiteLLM's expand_tool_permissions: expand each key to its servers and union the tools
    #[must_use]
    pub fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&BTreeMap<String, Vec<String>>>,
    ) -> BTreeMap<String, Vec<String>> {
        let Some(permissions) = tool_permissions else {
            return BTreeMap::new();
        };
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (identifier, tools) in permissions {
            let servers = self.expand_permission_list(from_ref(identifier));
            for server_id in servers {
                let entry = grouped.entry(server_id).or_default();
                for tool in tools {
                    if !entry.contains(tool) {
                        entry.push(tool.clone());
                    }
                }
            }
        }
        grouped
    }
}

// LiteLLM's alias, server_name or name match inside expand_permission_list
fn names_server(server: &MCPServer, identifier: &str) -> bool {
    let alias = server.fields.alias.as_deref() == Some(identifier);
    let server_name = server.fields.server_name.as_deref() == Some(identifier);
    alias || server_name || server.fields.name == identifier
}
