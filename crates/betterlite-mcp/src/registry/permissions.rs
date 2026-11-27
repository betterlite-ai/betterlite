use std::collections::BTreeMap;
use std::slice::from_ref;

use indexmap::IndexSet;
use tracing::debug;

use super::servers::ServerRegistry;
use crate::server_record::MCPServer;

pub trait PermissionExpansion {
    #[must_use]
    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String>;

    #[must_use]
    fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&BTreeMap<String, Vec<String>>>,
    ) -> BTreeMap<String, Vec<String>>;
}

impl PermissionExpansion for ServerRegistry {
    // LiteLLM's expand_permission_list: ids, then aliases and names, and anything else as given
    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String> {
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
            let matching: Vec<String> = registry
                .iter()
                .filter(|server| names_server(server, identifier))
                .map(|server| server.fields.server_id.clone())
                .collect();
            if matching.is_empty() {
                debug!(
                    "MCP permission entry {identifier:?} is not a known server; keeping it for the access check"
                );
                expanded.insert(identifier.clone());
            } else {
                expanded.extend(matching);
            }
        }
        expanded.into_iter().collect()
    }

    // LiteLLM's expand_tool_permissions: expand each key to its servers and union the tools
    fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&BTreeMap<String, Vec<String>>>,
    ) -> BTreeMap<String, Vec<String>> {
        let Some(permissions) = tool_permissions else {
            return BTreeMap::new();
        };
        let mut grouped: BTreeMap<String, IndexSet<String>> = BTreeMap::new();
        for (identifier, tools) in permissions {
            for server_id in self.expand_permission_list(from_ref(identifier)) {
                grouped
                    .entry(server_id)
                    .or_default()
                    .extend(tools.iter().cloned());
            }
        }
        grouped
            .into_iter()
            .map(|(server_id, tools)| (server_id, tools.into_iter().collect()))
            .collect()
    }
}

// LiteLLM's alias, server_name or name match inside expand_permission_list
fn names_server(server: &MCPServer, identifier: &str) -> bool {
    let alias = server.fields.alias.as_deref() == Some(identifier);
    let server_name = server.fields.server_name.as_deref() == Some(identifier);
    alias || server_name || server.fields.name == identifier
}
