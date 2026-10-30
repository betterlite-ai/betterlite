#![cfg(test)]

use super::resolve_target;
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::Target;

fn some_node() -> Vec<ClusterNodeConfig> {
    vec![ClusterNodeConfig {
        host: "a".to_owned(),
        port: 7000,
    }]
}

fn some_sentinel() -> Vec<(String, i64)> {
    vec![("a".to_owned(), 26379)]
}

#[test]
fn redis_target_resolution_prefers_cluster_over_everything_else() {
    let nodes = some_node();
    let sentinels = some_sentinel();
    let target = resolve_target(
        Some(&nodes),
        Some(&sentinels),
        Some("redis://host"),
        Some("host"),
    )
    .expect("startup_nodes names a target");
    assert_eq!(target, Target::Cluster);
}

#[test]
fn redis_target_resolution_prefers_url_over_sentinel_and_host() {
    let sentinels = some_sentinel();
    let target = resolve_target(None, Some(&sentinels), Some("redis://host"), Some("host"))
        .expect("sentinel_nodes names a target");
    assert_eq!(target, Target::Url);
}

#[test]
fn redis_target_resolution_prefers_url_over_host() {
    let target =
        resolve_target(None, None, Some("redis://host"), Some("host")).expect("url names a target");
    assert_eq!(target, Target::Url);
}

#[test]
fn redis_target_resolution_uses_sentinel_when_url_and_cluster_are_absent() {
    let sentinels = some_sentinel();
    let target = resolve_target(None, Some(&sentinels), None, Some("host"))
        .expect("sentinel_nodes names a target");
    assert_eq!(target, Target::Sentinel);
}

#[test]
fn redis_target_resolution_falls_back_to_direct_host() {
    let target = resolve_target(None, None, None, Some("host")).expect("host names a target");
    assert_eq!(target, Target::Direct);
}

#[test]
fn redis_target_resolution_errors_when_nothing_names_a_target() {
    resolve_target(None, None, None, None).unwrap_err();
}
