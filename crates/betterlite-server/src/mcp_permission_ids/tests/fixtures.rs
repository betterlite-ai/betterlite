pub(crate) mod database;
pub(crate) mod fake_rows;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use betterlite_mcp::server_record::MCPServer;
use tokio::sync::RwLock;

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the case value is JSON")
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

pub(crate) fn string_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

pub(crate) fn identifier_sets(entries: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    let mut resolved = BTreeMap::new();
    for (identifier, server_ids) in entries {
        resolved.insert(String::from(*identifier), string_set(server_ids));
    }
    resolved
}

// A stored grant document whose one entry nests the given number of arrays, for the depth bound.
pub(crate) fn nested_array_grant(arrays: usize) -> String {
    let opening = "[".repeat(arrays);
    let closing = "]".repeat(arrays);
    format!(r#"{{"srv-a": {opening}{closing}}}"#)
}

pub(crate) fn registry_server(text: &str) -> MCPServer {
    serde_json::from_str(text).expect("the registry record is an MCP server")
}

pub(crate) fn registry(
    config_servers: Vec<MCPServer>,
    db_servers: Vec<MCPServer>,
) -> SharedRegistry {
    let settings = RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: false,
        internal_ip_ranges: None,
    };
    Arc::new(RwLock::new(ServerRegistry::new(
        settings,
        config_servers,
        db_servers,
    )))
}
