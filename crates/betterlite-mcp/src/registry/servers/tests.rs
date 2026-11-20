#![cfg(test)]

use std::collections::BTreeSet;

use super::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

fn settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
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

fn allow_all_server(server_id: &str) -> MCPServer {
    let mut record = server(server_id);
    record.fields.allow_all_keys = true;
    record
}

fn ids(servers: &[&MCPServer]) -> Vec<String> {
    servers
        .iter()
        .map(|record| record.fields.server_id.clone())
        .collect()
}

#[test]
fn registry_new_keys_records_by_server_id() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a")],
        vec![server("test-db-a")],
    );
    assert_eq!(
        ids(&registry.get_registry()),
        ["test-config-a", "test-db-a"]
    );
    assert!(registry.is_config_declared_server("test-config-a"));
    assert!(!registry.is_config_declared_server("Name test-config-a"));
}

#[test]
fn registry_new_repeated_id_keeps_first_position_with_later_record() {
    let registry = ServerRegistry::new(
        settings(),
        vec![
            allow_all_server("test-config-a"),
            server("test-config-b"),
            server("test-config-a"),
        ],
        Vec::new(),
    );
    let served = registry.get_registry();
    assert_eq!(ids(&served), ["test-config-a", "test-config-b"]);
    assert!(!served[0].fields.allow_all_keys);
}

#[test]
fn registry_new_keeps_the_settings() {
    let registry = ServerRegistry::new(settings(), Vec::new(), Vec::new());
    assert_eq!(registry.settings, settings());
}

#[test]
fn registry_debug_renders_both_maps_and_the_settings() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a")],
        vec![server("test-db-a")],
    );
    let rendered = format!("{registry:?}");
    assert!(rendered.contains("test-config-a"));
    assert!(rendered.contains("test-db-a"));
    assert!(rendered.contains("public_hub_strict: true"));
}

#[test]
fn registry_get_registry_lists_config_records_before_database_records() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a"), server("test-config-b")],
        vec![server("test-db-a"), server("test-db-b")],
    );
    assert_eq!(
        ids(&registry.get_registry()),
        ["test-config-a", "test-config-b", "test-db-a", "test-db-b"]
    );
}

#[test]
fn registry_get_registry_keeps_shared_id_at_config_position_with_database_record() {
    let registry = ServerRegistry::new(
        settings(),
        vec![
            server("test-config-a"),
            server("test-config-shared"),
            server("test-config-b"),
        ],
        vec![allow_all_server("test-config-shared"), server("test-db-a")],
    );
    let served = registry.get_registry();
    assert_eq!(
        ids(&served),
        [
            "test-config-a",
            "test-config-shared",
            "test-config-b",
            "test-db-a"
        ]
    );
    assert!(served[1].fields.allow_all_keys);
}

#[test]
fn registry_empty_registry_answers_empty() {
    let registry = ServerRegistry::new(settings(), Vec::new(), Vec::new());
    assert!(registry.get_registry().is_empty());
    assert!(registry.get_all_mcp_server_ids().is_empty());
    assert!(registry.get_allow_all_keys_server_ids().is_empty());
    assert!(
        registry
            .get_mcp_servers_from_ids(&["test-config-a".to_owned()])
            .is_empty()
    );
    assert!(!registry.is_config_declared_server("test-config-a"));
}

#[test]
fn registry_all_ids_holds_every_record_once() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a"), server("test-config-shared")],
        vec![allow_all_server("test-config-shared"), server("test-db-a")],
    );
    let expected = BTreeSet::from([
        "test-config-a".to_owned(),
        "test-config-shared".to_owned(),
        "test-db-a".to_owned(),
    ]);
    assert_eq!(registry.get_all_mcp_server_ids(), expected);
}

#[test]
fn registry_allow_all_keys_lists_records_in_registry_order() {
    let registry = ServerRegistry::new(
        settings(),
        vec![allow_all_server("test-config-a"), server("test-config-b")],
        vec![allow_all_server("test-db-a"), server("test-db-b")],
    );
    assert_eq!(
        registry.get_allow_all_keys_server_ids(),
        ["test-config-a", "test-db-a"]
    );
}

#[test]
fn registry_is_config_declared_reads_only_the_config_map() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-shared")],
        vec![server("test-config-shared"), server("test-db-a")],
    );
    assert!(registry.is_config_declared_server("test-config-shared"));
    assert!(!registry.is_config_declared_server("test-db-a"));
    assert!(!registry.is_config_declared_server("test-unknown"));
}

#[test]
fn registry_get_mcp_servers_from_ids_follows_registry_order() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a"), server("test-config-b")],
        vec![server("test-db-a")],
    );
    let requested = vec!["test-db-a".to_owned(), "test-config-b".to_owned()];
    assert_eq!(
        ids(&registry.get_mcp_servers_from_ids(&requested)),
        ["test-config-b", "test-db-a"]
    );
}

#[test]
fn registry_get_mcp_servers_from_ids_skips_unknown_and_repeated_ids() {
    let registry = ServerRegistry::new(
        settings(),
        vec![server("test-config-a"), server("test-config-b")],
        vec![server("test-db-a")],
    );
    let requested = vec![
        "test-unknown".to_owned(),
        "test-config-a".to_owned(),
        "test-db-a".to_owned(),
        "test-config-a".to_owned(),
    ];
    assert_eq!(
        ids(&registry.get_mcp_servers_from_ids(&requested)),
        ["test-config-a", "test-db-a"]
    );
}
