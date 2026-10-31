#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

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
fn url_target_keeps_connection_kwargs_and_drops_client_or_cluster_kwargs() {
    let mut passthrough = passthrough_with(&[
        "ssl_cert_reqs",
        "single_connection_client",
        "decode_responses",
    ]);
    filter_passthrough_for_target(Target::Url, ClientKind::Sync, &mut passthrough)
        .expect("url target never rejects an unknown option");
    assert!(!passthrough.contains_key("ssl_cert_reqs"));
    assert!(!passthrough.contains_key("single_connection_client"));
    assert!(passthrough.contains_key("decode_responses"));
}

#[test]
fn non_cluster_kwargs_do_not_reach_a_cluster_target() {
    let mut passthrough = passthrough_with(&["host", "port", "startup_nodes", "max_connections"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Async, &mut passthrough)
        .expect("cluster target never rejects an unknown option");
    assert!(!passthrough.contains_key("host"));
    assert!(!passthrough.contains_key("port"));
    assert!(passthrough.contains_key("max_connections"));
}

#[test]
fn decode_responses_reaches_an_async_cluster_but_not_a_sync_cluster() {
    let mut async_passthrough = passthrough_with(&["decode_responses"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Async, &mut async_passthrough)
        .expect("cluster target never rejects an unknown option");
    assert!(async_passthrough.contains_key("decode_responses"));

    let mut sync_passthrough = passthrough_with(&["decode_responses"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Sync, &mut sync_passthrough)
        .expect("cluster target never rejects an unknown option");
    assert!(!sync_passthrough.contains_key("decode_responses"));
}

#[test]
fn direct_keeps_redis_tls_kwargs_and_sentinel_drops_cluster_only_kwargs() {
    let mut direct_passthrough = passthrough_with(&["ssl_ca_certs"]);
    filter_passthrough_for_target(Target::Direct, ClientKind::Sync, &mut direct_passthrough)
        .expect("ssl_ca_certs is a recognized direct option");
    assert!(direct_passthrough.contains_key("ssl_ca_certs"));

    let mut sentinel_passthrough = passthrough_with(&["reinitialize_steps"]);
    filter_passthrough_for_target(
        Target::Sentinel,
        ClientKind::Async,
        &mut sentinel_passthrough,
    )
    .expect("sentinel target never rejects an unknown option");
    assert!(sentinel_passthrough.is_empty());
}

#[test]
fn direct_target_rejects_an_unrecognized_option() {
    let mut passthrough = passthrough_with(&["not_a_real_option"]);
    let error = filter_passthrough_for_target(Target::Direct, ClientKind::Sync, &mut passthrough)
        .expect_err("an unknown name on a direct connection must be rejected");
    assert!(error.to_string().contains("not_a_real_option"));
}

#[test]
fn direct_allowlist_matches_the_selected_client_constructor() {
    let mut sync = passthrough_with(&["auto_close_connection_pool"]);
    filter_passthrough_for_target(Target::Direct, ClientKind::Sync, &mut sync)
        .expect("sync Redis declares auto_close_connection_pool");

    let mut async_client = passthrough_with(&["auto_close_connection_pool"]);
    assert!(
        filter_passthrough_for_target(Target::Direct, ClientKind::Async, &mut async_client)
            .is_err()
    );

    let mut asynchronous_ocsp = passthrough_with(&["ssl_validate_ocsp"]);
    filter_passthrough_for_target(Target::Direct, ClientKind::Async, &mut asynchronous_ocsp)
        .expect("async Redis declares ssl_validate_ocsp");

    let mut synchronous_ocsp = passthrough_with(&["ssl_validate_ocsp"]);
    assert!(
        filter_passthrough_for_target(Target::Direct, ClientKind::Sync, &mut synchronous_ocsp)
            .is_err()
    );
}

#[rstest]
#[case::url(Target::Url)]
#[case::cluster(Target::Cluster)]
#[case::sentinel(Target::Sentinel)]
fn url_cluster_and_sentinel_targets_silently_drop_an_unrecognized_option(#[case] target: Target) {
    let mut passthrough = passthrough_with(&["not_a_real_option"]);
    filter_passthrough_for_target(target, ClientKind::Sync, &mut passthrough)
        .expect("only a direct connection rejects an unknown option");
    assert!(passthrough.is_empty());
}

#[test]
fn ssl_validate_ocsp_is_dropped_from_both_sync_and_async_cluster_targets() {
    let mut sync_passthrough = passthrough_with(&["ssl_validate_ocsp"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Sync, &mut sync_passthrough)
        .expect("cluster target never rejects an unknown option");
    assert!(!sync_passthrough.contains_key("ssl_validate_ocsp"));

    let mut async_passthrough = passthrough_with(&["ssl_validate_ocsp"]);
    filter_passthrough_for_target(Target::Cluster, ClientKind::Async, &mut async_passthrough)
        .expect("cluster target never rejects an unknown option");
    assert!(!async_passthrough.contains_key("ssl_validate_ocsp"));
}

#[test]
fn client_name_and_socket_read_size_reach_a_url_target() {
    let mut passthrough = passthrough_with(&["client_name", "socket_read_size"]);
    filter_passthrough_for_target(Target::Url, ClientKind::Sync, &mut passthrough)
        .expect("url target never rejects an unknown option");
    assert!(passthrough.contains_key("client_name"));
    assert!(passthrough.contains_key("socket_read_size"));
}
