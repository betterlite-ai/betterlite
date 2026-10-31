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

#[rstest]
#[case::sync_ocsp(Target::Direct, ClientKind::Sync, "ssl_validate_ocsp", true)]
#[case::async_ocsp(Target::Direct, ClientKind::Async, "ssl_validate_ocsp", false)]
#[case::sync_pool(Target::Direct, ClientKind::Sync, "auto_close_connection_pool", false)]
#[case::async_pool(Target::Direct, ClientKind::Async, "auto_close_connection_pool", true)]
#[case::sync_stapled(Target::Direct, ClientKind::Sync, "ssl_validate_ocsp_stapled", true)]
#[case::async_stapled(Target::Direct, ClientKind::Async, "ssl_validate_ocsp_stapled", false)]
#[case::sync_context(Target::Direct, ClientKind::Sync, "ssl_ocsp_context", true)]
#[case::async_context(Target::Direct, ClientKind::Async, "ssl_ocsp_context", false)]
#[case::sync_cert(Target::Direct, ClientKind::Sync, "ssl_ocsp_expected_cert", true)]
#[case::async_cert(Target::Direct, ClientKind::Async, "ssl_ocsp_expected_cert", false)]
#[case::sync_packer(Target::Url, ClientKind::Sync, "command_packer", true)]
#[case::async_packer(Target::Url, ClientKind::Async, "command_packer", false)]
#[case::sync_encoder(Target::Url, ClientKind::Sync, "encoder_class", false)]
#[case::async_encoder(Target::Url, ClientKind::Async, "encoder_class", true)]
#[case::sentinel_async_pool(
    Target::Sentinel,
    ClientKind::Async,
    "auto_close_connection_pool",
    false
)]
#[case::sentinel_async_cache(Target::Sentinel, ClientKind::Async, "cache", true)]
#[case::sentinel_async_cache_config(Target::Sentinel, ClientKind::Async, "cache_config", true)]
#[case::sentinel_async_ocsp(Target::Sentinel, ClientKind::Async, "ssl_validate_ocsp", true)]
#[case::sentinel_async_stapled(
    Target::Sentinel,
    ClientKind::Async,
    "ssl_validate_ocsp_stapled",
    true
)]
#[case::sentinel_async_context(Target::Sentinel, ClientKind::Async, "ssl_ocsp_context", true)]
#[case::sentinel_async_cert(Target::Sentinel, ClientKind::Async, "ssl_ocsp_expected_cert", true)]
#[case::sentinel_async_maintenance(
    Target::Sentinel,
    ClientKind::Async,
    "oss_cluster_maint_notifications_handler",
    true
)]
fn redis_target_resolution_client_allowlist(
    #[case] target: Target,
    #[case] client: ClientKind,
    #[case] name: &str,
    #[case] accepted: bool,
) {
    let mut options = passthrough_with(&[name]);
    assert_eq!(
        filter_passthrough_for_target(target, client, &mut options).is_ok()
            && options.contains_key(name),
        accepted
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
