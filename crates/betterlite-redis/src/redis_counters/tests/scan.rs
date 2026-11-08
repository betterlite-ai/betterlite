use rstest::rstest;

use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

use super::support::{ADMISSION, cluster_pair, counters, direct_settings, transport};

fn scan_reply(cursor: &str, keys: &[&str]) -> Reply {
    Reply::Array(vec![
        Reply::Bulk(cursor.to_owned()),
        Reply::Array(
            keys.iter()
                .map(|key| Reply::Bulk((*key).to_owned()))
                .collect(),
        ),
    ])
}

#[tokio::test]
async fn async_scan_iter_follows_the_cursor_until_zero() {
    let server = FakeServer::start().await;
    server.script("SCAN", scan_reply("9", &["ns:one"]));
    server.script("SCAN", scan_reply("0", &["ns:two"]));
    let (counters, hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let keys = counters
        .async_scan_iter("prefix", 10)
        .await
        .expect("the keys");
    assert_eq!(keys, vec![b"ns:one".to_vec(), b"ns:two".to_vec()]);
    let scans = server.commands_named("SCAN");
    assert_eq!(scans.len(), 2);
    assert_eq!(
        scans[0].args,
        ["SCAN", "0", "MATCH", "ns:prefix*", "COUNT", "10"]
    );
    assert_eq!(
        scans[1].args,
        ["SCAN", "9", "MATCH", "ns:prefix*", "COUNT", "10"]
    );
    assert_eq!(hooks.success_names(), vec!["async_scan_iter"]);
    assert_eq!(gate.enters(), vec!["async_scan_iter"]);
    assert_eq!(gate.exits(), vec![ADMISSION]);
}

#[tokio::test]
async fn async_scan_iter_stops_mid_batch_at_the_count() {
    let server = FakeServer::start().await;
    server.script("SCAN", scan_reply("7", &["one", "two", "three"]));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let keys = counters
        .async_scan_iter("prefix", 2)
        .await
        .expect("the keys");
    assert_eq!(keys, vec![b"one".to_vec(), b"two".to_vec()]);
    assert_eq!(server.commands_named("SCAN").len(), 1);
}

#[rstest]
#[case::bare("prefix", "ns:prefix*")]
#[case::already_namespaced("ns:prefix", "ns:prefix*")]
#[case::already_a_glob("ns:pre*", "ns:pre**")]
#[tokio::test]
async fn async_scan_iter_namespaces_the_pattern_then_appends_a_glob(
    #[case] pattern: &str,
    #[case] expected: &str,
) {
    let server = FakeServer::start().await;
    server.script("SCAN", scan_reply("0", &[]));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        Some("ns"),
    );
    let keys = counters
        .async_scan_iter(pattern, 5)
        .await
        .expect("the keys");
    assert!(keys.is_empty());
    assert_eq!(
        server.commands_named("SCAN")[0].args,
        ["SCAN", "0", "MATCH", expected, "COUNT", "5"]
    );
}

#[tokio::test]
async fn async_scan_iter_reports_a_failure_hook_and_the_error() {
    let server = FakeServer::start().await;
    server.script("SCAN", Reply::Error("ERR scan failed".to_owned()));
    let (counters, hooks, gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Standalone),
        None,
    );
    let error = counters
        .async_scan_iter("prefix", 5)
        .await
        .expect_err("the scan error");
    assert!(matches!(error, RedisCountersError::Client(_)), "{error}");
    assert!(error.to_string().contains("scan failed"), "{error}");
    assert_eq!(hooks.failure_names(), vec!["async_scan_iter"]);
    assert_eq!(gate.failures().len(), 1);
    assert!(gate.exits().is_empty());
}

#[tokio::test]
async fn a_cluster_scan_walks_each_primary_by_address() {
    let pair = cluster_pair().await;
    pair.first.script("SCAN", scan_reply("0", &["ns:one"]));
    pair.second.script("SCAN", scan_reply("0", &["ns:two"]));
    let (counters, _hooks, _gate) = counters(pair.transport, Some("ns"));
    let keys = counters
        .async_scan_iter("prefix", 10)
        .await
        .expect("the keys");
    assert_eq!(keys, vec![b"ns:one".to_vec(), b"ns:two".to_vec()]);
    assert_eq!(pair.first.commands_named("SCAN").len(), 1);
    assert_eq!(pair.second.commands_named("SCAN").len(), 1);
    assert_eq!(
        pair.first.commands_named("SCAN")[0].args,
        ["SCAN", "0", "MATCH", "ns:prefix*", "COUNT", "10"]
    );
    assert_eq!(
        pair.second.commands_named("SCAN")[0].args,
        ["SCAN", "0", "MATCH", "ns:prefix*", "COUNT", "10"]
    );
}

#[tokio::test]
async fn a_cluster_scan_shares_the_count_cap_across_primaries() {
    let pair = cluster_pair().await;
    pair.first.script("SCAN", scan_reply("0", &["one"]));
    pair.second.script("SCAN", scan_reply("0", &["two"]));
    let (counters, _hooks, _gate) = counters(pair.transport, None);
    let keys = counters
        .async_scan_iter("prefix", 1)
        .await
        .expect("the keys");
    assert_eq!(keys, vec![b"one".to_vec()]);
    assert_eq!(pair.first.commands_named("SCAN").len(), 1);
    assert!(pair.second.commands_named("SCAN").is_empty());
}

#[tokio::test]
async fn a_cluster_family_with_a_standalone_client_scans_one_connection() {
    let server = FakeServer::start().await;
    server.script("SCAN", scan_reply("0", &["one"]));
    let (counters, _hooks, _gate) = counters(
        transport(direct_settings(&server), RedisCacheFamily::Cluster),
        None,
    );
    let keys = counters
        .async_scan_iter("prefix", 5)
        .await
        .expect("the keys");
    assert_eq!(keys, vec![b"one".to_vec()]);
    assert_eq!(server.commands_named("SCAN").len(), 1);
    assert!(server.commands_named("CLUSTER").is_empty());
}
