#![cfg(test)]

use super::{IpFilteredIds, filter_server_ids_by_ip, filter_server_ids_by_ip_with_info};
use crate::registry::servers::{RegistrySettings, ServerRegistry};
use crate::server_record::MCPServer;

const PUBLIC: &str = r#"{"server_id":"test-public","name":"Public","transport":"http"}"#;
const SECOND_PUBLIC: &str = r#"{"server_id":"test-public-b","name":"Public B","transport":"http"}"#;
const PRIVATE: &str = r#"{"server_id":"test-private","name":"Private","transport":"http","available_on_public_internet":false}"#;

const INTERNAL_IP: &str = "10.0.0.5";
const EXTERNAL_IP: &str = "203.0.113.7";

fn settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: true,
        internal_ip_ranges: None,
    }
}

fn record(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn registry(json_records: &[&str]) -> ServerRegistry {
    let mut servers = Vec::new();
    for json in json_records {
        servers.push(record(json));
    }
    ServerRegistry::new(settings(), servers, Vec::new())
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn no_ip_returns_the_input_ids_in_order() {
    let loaded = registry(&[PRIVATE, PUBLIC]);
    let requested = ids(&["test-private", "test-ghost", "test-public"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, None);
    assert_eq!(
        filtered,
        IpFilteredIds {
            server_ids: requested.clone(),
            ip_blocked_count: 0
        }
    );
    assert_eq!(
        filter_server_ids_by_ip(&loaded, &requested, None),
        requested
    );
}

#[test]
fn no_ip_keeps_a_private_server_the_external_ip_cannot_reach() {
    let loaded = registry(&[PRIVATE]);
    let requested = ids(&["test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, None);
    assert_eq!(filtered.server_ids, requested);
    assert_eq!(filtered.ip_blocked_count, 0);
}

#[test]
fn no_ip_empty_input_stays_empty() {
    let loaded = registry(&[PUBLIC]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &[], None);
    assert!(filtered.server_ids.is_empty());
    assert_eq!(filtered.ip_blocked_count, 0);
    assert!(filter_server_ids_by_ip(&loaded, &[], None).is_empty());
}

#[test]
fn no_ip_repeated_ids_stay_repeated() {
    let loaded = registry(&[PRIVATE]);
    let requested = ids(&["test-private", "test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, None);
    assert_eq!(filtered.server_ids, requested);
    assert_eq!(filtered.ip_blocked_count, 0);
}

#[test]
fn ip_keeps_reachable_servers_in_input_order() {
    let loaded = registry(&[PUBLIC, SECOND_PUBLIC]);
    let requested = ids(&["test-public-b", "test-public"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert_eq!(filtered.server_ids, requested);
    assert_eq!(filtered.ip_blocked_count, 0);
}

#[test]
fn ip_drops_a_private_server_and_counts_it() {
    let loaded = registry(&[PRIVATE]);
    let requested = ids(&["test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert!(filtered.server_ids.is_empty());
    assert_eq!(filtered.ip_blocked_count, 1);
}

#[test]
fn ip_internal_address_reaches_a_private_server() {
    let loaded = registry(&[PRIVATE]);
    let requested = ids(&["test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(INTERNAL_IP));
    assert_eq!(filtered.server_ids, requested);
    assert_eq!(filtered.ip_blocked_count, 0);
}

#[test]
fn ip_public_server_is_reachable_from_both_addresses() {
    let loaded = registry(&[PUBLIC]);
    let requested = ids(&["test-public"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(INTERNAL_IP));
    assert_eq!(
        filtered,
        IpFilteredIds {
            server_ids: requested.clone(),
            ip_blocked_count: 0
        }
    );
    assert_eq!(
        filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP)),
        IpFilteredIds {
            server_ids: requested,
            ip_blocked_count: 0
        }
    );
}

#[test]
fn ip_unknown_id_is_dropped_without_counting() {
    let loaded = registry(&[PUBLIC]);
    let requested = ids(&["test-ghost"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert!(filtered.server_ids.is_empty());
    assert_eq!(filtered.ip_blocked_count, 0);
    assert!(filter_server_ids_by_ip(&loaded, &requested, Some(INTERNAL_IP)).is_empty());
}

#[test]
fn ip_counts_each_occurrence_of_a_blocked_id() {
    let loaded = registry(&[PRIVATE]);
    let requested = ids(&["test-private", "test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert!(filtered.server_ids.is_empty());
    assert_eq!(filtered.ip_blocked_count, 2);
}

#[test]
fn ip_mixed_input_keeps_order_and_counts_only_blocked_known_ids() {
    let loaded = registry(&[PUBLIC, PRIVATE]);
    let requested = ids(&["test-ghost", "test-private", "test-public", "test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert_eq!(filtered.server_ids, ids(&["test-public"]));
    assert_eq!(filtered.ip_blocked_count, 2);
}

#[test]
fn ip_filter_server_ids_by_ip_matches_the_info_form() {
    let loaded = registry(&[PUBLIC, PRIVATE]);
    let requested = ids(&["test-public", "test-ghost", "test-private"]);
    let blocked = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(EXTERNAL_IP));
    assert_eq!(
        filter_server_ids_by_ip(&loaded, &requested, Some(EXTERNAL_IP)),
        blocked.server_ids
    );
    let kept = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(INTERNAL_IP));
    assert_eq!(
        filter_server_ids_by_ip(&loaded, &requested, Some(INTERNAL_IP)),
        kept.server_ids
    );
}

#[test]
fn ip_empty_client_ip_still_filters() {
    let loaded = registry(&[PUBLIC, PRIVATE]);
    let requested = ids(&["test-public", "test-private"]);
    let filtered = filter_server_ids_by_ip_with_info(&loaded, &requested, Some(""));
    assert_eq!(filtered.server_ids, ids(&["test-public"]));
    assert_eq!(filtered.ip_blocked_count, 1);
}
