use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_db::mcp_stored_contracts::MCPTransport;

use super::servers::ServerRegistry;
use crate::server_payload::rules::is_mcp_stdio_blocked;
use crate::server_record::MCPServer;
use crate::server_utils::names::known_tool_name_spellings;
use crate::server_utils::prefixes::{
    PrefixPolicy, iter_known_server_prefixes, match_known_server_prefix,
};

mod tests;

// LiteLLM's tool_name_to_mcp_server_name_mapping, which the catalog code fills (B4682)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolOwners {
    pub mapping: BTreeMap<String, String>,
}

impl ServerRegistry {
    // LiteLLM's _known_prefix_to_server: a form two servers share stays with the first registered
    #[must_use]
    pub fn known_prefix_to_server(&self, policy: &PrefixPolicy) -> BTreeMap<String, &MCPServer> {
        let mut prefix_to_server = BTreeMap::new();
        for server in self.get_registry().into_iter().rev() {
            for prefix in iter_known_server_prefixes(server, policy) {
                prefix_to_server.insert(prefix.replace(' ', "_"), server);
            }
        }
        prefix_to_server
    }

    // LiteLLM's server_exposes_tool: a spelling of the tool name whose owner this server is
    #[must_use]
    pub fn server_exposes_tool(
        server: &MCPServer,
        tool_name: &str,
        owners: &ToolOwners,
        policy: &PrefixPolicy,
    ) -> bool {
        let prefixes = iter_known_server_prefixes(server, policy);
        let mut owned: BTreeSet<String> = prefixes
            .iter()
            .map(|prefix| prefix.replace(' ', "_"))
            .collect();
        if !server.fields.name.is_empty() {
            owned.insert(server.fields.name.replace(' ', "_"));
        }
        let spellings = known_tool_name_spellings(tool_name, &prefixes, &policy.separator);
        spellings.iter().any(|spelling| {
            owners
                .mapping
                .get(spelling)
                .is_some_and(|owner| owned.contains(&owner.replace(' ', "_")))
        })
    }

    // LiteLLM's _get_mcp_server_from_tool_name: the owner map first, then the known prefixes
    #[must_use]
    pub fn server_for_tool_name(
        &self,
        tool_name: &str,
        owners: &ToolOwners,
        policy: &PrefixPolicy,
        stdio: &StdioPolicy,
    ) -> Option<&MCPServer> {
        let registry = self.get_registry();
        let prefix_to_server = self.known_prefix_to_server(policy);
        if let Some(owner) = owners.mapping.get(tool_name) {
            let normalised = owner.replace(' ', "_");
            if let Some(server) = prefix_to_server.get(&normalised) {
                return Some(*server);
            }
            let by_name = registry
                .iter()
                .find(|server| server.fields.name.replace(' ', "_") == normalised)
                .copied();
            if let Some(server) = by_name {
                return Some(server);
            }
        }
        let prefixes: Vec<String> = prefix_to_server.keys().cloned().collect();
        let (prefix, original_tool_name) = match_known_server_prefix(tool_name, &prefixes, policy)?;
        let server = prefix_to_server.get(&prefix).copied()?;
        let exposed = Self::server_exposes_tool(server, &original_tool_name, owners, policy);
        let stdio_blocked = server.fields.transport == MCPTransport::Stdio
            && is_mcp_stdio_blocked(Some("stdio"), stdio);
        (exposed || stdio_blocked).then_some(server)
    }

    // LiteLLM's server_owning_tool_name_prefix
    #[must_use]
    pub fn server_owning_prefix(
        &self,
        tool_name: &str,
        policy: &PrefixPolicy,
    ) -> Option<&MCPServer> {
        let prefix_to_server = self.known_prefix_to_server(policy);
        let prefixes: Vec<String> = prefix_to_server.keys().cloned().collect();
        let matched = match_known_server_prefix(tool_name, &prefixes, policy)?;
        let prefix = matched.0;
        prefix_to_server.get(&prefix).copied()
    }
}
