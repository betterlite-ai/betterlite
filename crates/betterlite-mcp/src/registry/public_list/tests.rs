#![cfg(test)]

use crate::registry::public_servers::PublicServers as _;
use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

fn settings(public_mcp_servers: &[&str]) -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: public_mcp_servers
            .iter()
            .map(|server_id| (*server_id).to_owned())
            .collect(),
        public_hub_strict: true,
        internal_ip_ranges: None,
    }
}

fn server(server_id: &str) -> MCPServer {
    let json = format!(
        r#"{{"server_id": "{server_id}", "name": "Name {server_id}", "transport": "http"}}"#
    );
    serde_json::from_str(&json).unwrap()
}

#[test]
fn public_list_reads_empty_when_unset() {
    let registry = ServerRegistry::new(settings(&[]), Vec::new(), Vec::new());
    assert!(registry.public_mcp_servers().is_empty());
}

#[test]
fn public_list_reads_configured_order() {
    let registry = ServerRegistry::new(
        settings(&["test-beta", "test-alpha"]),
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        registry.public_mcp_servers(),
        ["test-beta".to_owned(), "test-alpha".to_owned()].as_slice()
    );
}

#[test]
fn public_list_set_replaces_and_keeps_order() {
    let mut registry = ServerRegistry::new(settings(&["test-old"]), Vec::new(), Vec::new());
    registry.set_public_mcp_servers(vec![
        "test-beta".to_owned(),
        "test-alpha".to_owned(),
        "test-beta".to_owned(),
    ]);
    assert_eq!(
        registry.public_mcp_servers(),
        [
            "test-beta".to_owned(),
            "test-alpha".to_owned(),
            "test-beta".to_owned()
        ]
        .as_slice()
    );
}

#[test]
fn public_list_set_empty_clears() {
    let mut registry = ServerRegistry::new(settings(&["test-alpha"]), Vec::new(), Vec::new());
    registry.set_public_mcp_servers(Vec::new());
    assert!(registry.public_mcp_servers().is_empty());
}

#[test]
fn public_list_set_reaches_is_public() {
    let mut registry =
        ServerRegistry::new(settings(&[]), vec![server("test-config-a")], Vec::new());
    assert!(!registry.is_server_public("test-config-a", None));
    registry.set_public_mcp_servers(vec!["test-config-a".to_owned()]);
    assert!(registry.is_server_public("test-config-a", None));
    registry.set_public_mcp_servers(Vec::new());
    assert!(!registry.is_server_public("test-config-a", None));
}
