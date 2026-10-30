#![cfg(test)]

use std::collections::BTreeMap;

use super::filter_passthrough_for_target;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{ClientKind, Target};

fn passthrough_with(names: &[&str]) -> BTreeMap<String, PassthroughValue> {
    names
        .iter()
        .map(|name| ((*name).to_owned(), PassthroughValue::Bool(true)))
        .collect()
}

#[test]
fn cluster_only_kwargs_do_not_reach_a_url_target() {
    let mut passthrough =
        passthrough_with(&["ssl_cert_reqs", "require_full_coverage", "socket_timeout"]);
    filter_passthrough_for_target(Target::Url, ClientKind::Sync, &mut passthrough);
    assert!(!passthrough.contains_key("ssl_cert_reqs"));
    assert!(!passthrough.contains_key("require_full_coverage"));
    assert!(passthrough.contains_key("socket_timeout"));
}

#[test]
fn non_cluster_kwargs_do_not_reach_a_cluster_target() {
    let mut passthrough = passthrough_with(&["host", "port", "startup_nodes", "max_connections"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Async, &mut passthrough);
    assert!(!passthrough.contains_key("host"));
    assert!(!passthrough.contains_key("port"));
    assert!(passthrough.contains_key("max_connections"));
}

#[test]
fn decode_responses_reaches_an_async_cluster_but_not_a_sync_cluster() {
    let mut async_passthrough = passthrough_with(&["decode_responses"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Async, &mut async_passthrough);
    assert!(async_passthrough.contains_key("decode_responses"));

    let mut sync_passthrough = passthrough_with(&["decode_responses"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Sync, &mut sync_passthrough);
    assert!(!sync_passthrough.contains_key("decode_responses"));
}

#[test]
fn direct_and_sentinel_targets_also_drop_cluster_only_kwargs() {
    let mut direct_passthrough = passthrough_with(&["ssl_ca_certs"]);
    filter_passthrough_for_target(Target::Direct, ClientKind::Sync, &mut direct_passthrough);
    assert!(direct_passthrough.is_empty());

    let mut sentinel_passthrough = passthrough_with(&["reinitialize_steps"]);
    filter_passthrough_for_target(
        Target::Sentinel,
        ClientKind::Async,
        &mut sentinel_passthrough,
    );
    assert!(sentinel_passthrough.is_empty());
}
