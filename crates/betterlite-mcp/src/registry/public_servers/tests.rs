#![cfg(test)]

use super::PublicServers as _;
use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

fn settings(public_mcp_servers: &[&str], public_hub_strict: bool) -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: public_mcp_servers
            .iter()
            .map(|server_id| (*server_id).to_owned())
            .collect(),
        public_hub_strict,
        internal_ip_ranges: None,
    }
}

fn server(server_id: &str) -> MCPServer {
    let json = format!(
        r#"{{"server_id": "{server_id}", "name": "Name {server_id}", "transport": "http"}}"#
    );
    serde_json::from_str(&json).unwrap()
}

fn private_server(server_id: &str) -> MCPServer {
    let mut record = server(server_id);
    record.fields.available_on_public_internet = false;
    record
}

fn ids(servers: &[&MCPServer]) -> Vec<String> {
    servers
        .iter()
        .map(|record| record.fields.server_id.clone())
        .collect()
}

#[test]
fn is_public_listed_server_with_strict_on_is_public() {
    let registry = ServerRegistry::new(
        settings(&["test-config-a"], true),
        vec![private_server("test-config-a")],
        Vec::new(),
    );
    assert!(registry.is_server_public("test-config-a", None));
}

#[test]
fn is_public_unlisted_public_internet_server_with_strict_on_is_private() {
    let registry = ServerRegistry::new(
        settings(&[], true),
        vec![server("test-config-a")],
        Vec::new(),
    );
    assert!(!registry.is_server_public("test-config-a", None));
}

#[test]
fn is_public_unlisted_public_internet_server_with_strict_off_is_public() {
    let registry = ServerRegistry::new(settings(&[], false), Vec::new(), vec![server("test-db-a")]);
    assert!(registry.is_server_public("test-db-a", None));
}

#[test]
fn is_public_unlisted_private_server_with_strict_off_is_private() {
    let registry = ServerRegistry::new(
        settings(&[], false),
        Vec::new(),
        vec![private_server("test-db-a")],
    );
    assert!(!registry.is_server_public("test-db-a", None));
}

#[test]
fn is_public_unknown_listed_id_is_private() {
    let registry = ServerRegistry::new(
        settings(&["test-unknown"], false),
        vec![server("test-config-a")],
        Vec::new(),
    );
    assert!(!registry.is_server_public("test-unknown", None));
}

#[test]
fn is_public_reads_the_database_record_for_a_shared_id() {
    let registry = ServerRegistry::new(
        settings(&[], false),
        vec![server("test-shared")],
        vec![private_server("test-shared")],
    );
    assert!(!registry.is_server_public("test-shared", None));
}

#[test]
fn is_public_explicit_empty_list_overrides_the_settings_list() {
    let registry = ServerRegistry::new(
        settings(&["test-config-a"], true),
        vec![server("test-config-a")],
        Vec::new(),
    );
    assert!(!registry.is_server_public("test-config-a", Some(&[])));
}

#[test]
fn is_public_explicit_list_replaces_the_settings_list() {
    let registry = ServerRegistry::new(
        settings(&["test-config-a"], true),
        vec![server("test-config-a"), server("test-config-b")],
        Vec::new(),
    );
    let published_ids = vec!["test-config-b".to_owned()];
    assert!(!registry.is_server_public("test-config-a", Some(&published_ids)));
    assert!(registry.is_server_public("test-config-b", Some(&published_ids)));
}

#[test]
fn list_strict_default_keeps_only_listed_ids() {
    let registry = ServerRegistry::new(
        settings(&["test-config-b", "test-db-b"], true),
        vec![server("test-config-a"), server("test-config-b")],
        vec![server("test-db-a"), server("test-db-b")],
    );
    assert_eq!(
        ids(&registry.public_servers()),
        ["test-config-b", "test-db-b"]
    );
}

#[test]
fn list_legacy_mode_adds_unlisted_public_internet_servers() {
    let registry = ServerRegistry::new(
        settings(&["test-config-a"], false),
        vec![
            server("test-config-a"),
            private_server("test-config-b"),
            server("test-config-c"),
        ],
        vec![private_server("test-db-a"), server("test-db-b")],
    );
    assert_eq!(
        ids(&registry.public_servers()),
        ["test-config-a", "test-config-c", "test-db-b"]
    );
}

#[test]
fn list_keeps_registry_order() {
    let registry = ServerRegistry::new(
        settings(&["test-db-b", "test-config-b", "test-db-a"], true),
        vec![server("test-config-a"), server("test-config-b")],
        vec![server("test-db-a"), server("test-db-b")],
    );
    assert_eq!(
        ids(&registry.public_servers()),
        ["test-config-b", "test-db-a", "test-db-b"]
    );
}

#[test]
fn list_empty_registry_and_empty_list_give_nothing() {
    let registry = ServerRegistry::new(settings(&[], false), Vec::new(), Vec::new());
    assert!(registry.public_servers().is_empty());
}

#[test]
fn list_empty_list_with_strict_on_gives_nothing() {
    let registry = ServerRegistry::new(
        settings(&[], true),
        vec![server("test-config-a")],
        vec![server("test-db-a")],
    );
    assert!(registry.public_servers().is_empty());
}

#[test]
fn list_reads_the_database_record_for_a_shared_id() {
    let registry = ServerRegistry::new(
        settings(&[], false),
        vec![server("test-shared")],
        vec![private_server("test-shared")],
    );
    assert!(registry.public_servers().is_empty());
}
