use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use redis::Value;
use redis::cmd;

use super::fake_server::{ClusterSlots, FakeServer, Reply};
use super::fixtures::{cluster_pair, cluster_record, factory, key_in_slot_range, redis_slot};
use crate::redis_client_factories::RedisClient;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_live_client::LiveTarget;

fn topology(primary: &FakeServer, replica: &FakeServer) -> ClusterSlots {
    vec![(
        0,
        16383,
        vec![
            (primary.host(), primary.port()),
            (replica.host(), replica.port()),
        ],
    )]
}

#[tokio::test]
async fn a_cluster_routes_each_key_to_the_node_owning_its_slot() {
    let (first, second) = cluster_pair().await;
    let record = cluster_record(&[&first]);
    let client = factory(None)
        .open_async(&RedisClient::Cluster(record), None)
        .await
        .expect("a cluster connection");
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    client
        .query::<Value>(cmd("SET").arg(low).arg("1"))
        .await
        .expect("a low slot write");
    client
        .query::<Value>(cmd("SET").arg(high).arg("2"))
        .await
        .expect("a high slot write");
    assert!(
        first
            .commands_named("SET")
            .iter()
            .any(|recorded| recorded.args[1] == low),
        "the first node owns the low slot"
    );
    assert!(
        second
            .commands_named("SET")
            .iter()
            .any(|recorded| recorded.args[1] == high),
        "the second node owns the high slot"
    );
    assert!(
        !first
            .commands_named("SET")
            .iter()
            .any(|recorded| recorded.args[1] == high)
    );
}

#[tokio::test]
async fn a_cluster_mget_returns_values_in_input_order_across_slots() {
    let (first, second) = cluster_pair().await;
    first.script("MGET", Reply::EchoArgs);
    second.script("MGET", Reply::EchoArgs);
    let client = factory(None)
        .open_async(&RedisClient::Cluster(cluster_record(&[&first])), None)
        .await
        .expect("a cluster connection");
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    let values: Vec<String> = client
        .query(cmd("MGET").arg(low).arg(high))
        .await
        .expect("a cross-slot MGET");
    assert_eq!(values, [low.to_owned(), high.to_owned()]);
    assert_eq!(first.commands_named("MGET").len(), 1);
    assert_eq!(second.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn a_cluster_keeps_the_response_and_connection_timeouts() {
    let (first, second) = cluster_pair().await;
    first.silence("GET");
    second.silence("GET");
    let mut record = cluster_record(&[&first]);
    record.socket_timeout = Some(0.2_f64);
    record.socket_connect_timeout = Some(0.4_f64);
    let client = factory(None)
        .open_async(&RedisClient::Cluster(record.clone()), None)
        .await
        .expect("a cluster connection");
    let key = key_in_slot_range(0, 8191);
    let started = Instant::now();
    let _timed_out = client.query::<Value>(cmd("GET").arg(key)).await;
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Cluster(record))
        .expect("a cluster endpoint");
    assert_eq!(endpoint.response_timeout, Duration::from_millis(200));
    assert_eq!(endpoint.connect_timeout, Duration::from_millis(400));
    assert!(matches!(endpoint.target, LiveTarget::Cluster(_)));
}

#[tokio::test]
async fn read_from_replicas_sends_reads_to_a_replica() {
    let primary = FakeServer::start().await;
    let replica = FakeServer::start().await;
    primary.set_cluster_slots(topology(&primary, &replica));
    replica.set_cluster_slots(topology(&primary, &replica));
    let mut record = cluster_record(&[&primary]);
    record.passthrough = BTreeMap::from([(
        "read_from_replicas".to_owned(),
        PassthroughValue::Bool(true),
    )]);
    let client = factory(None)
        .open_async(&RedisClient::Cluster(record), None)
        .await
        .expect("a cluster connection");
    let key = key_in_slot_range(0, 16383);
    client
        .query::<Value>(cmd("GET").arg(key))
        .await
        .expect("a read");
    assert_eq!(
        replica.commands_named("GET").len(),
        1,
        "the read reached the replica"
    );
    assert_eq!(
        primary.commands_named("GET").len(),
        0,
        "the primary served no read"
    );
}

#[tokio::test]
async fn a_cluster_without_read_from_replicas_reads_from_the_primary() {
    let primary = FakeServer::start().await;
    let replica = FakeServer::start().await;
    primary.set_cluster_slots(topology(&primary, &replica));
    replica.set_cluster_slots(topology(&primary, &replica));
    let client = factory(None)
        .open_async(&RedisClient::Cluster(cluster_record(&[&primary])), None)
        .await
        .expect("a cluster connection");
    let key = key_in_slot_range(0, 16383);
    client
        .query::<Value>(cmd("GET").arg(key))
        .await
        .expect("a read");
    assert_eq!(primary.commands_named("GET").len(), 1);
    assert_eq!(replica.commands_named("GET").len(), 0);
}

#[test]
fn the_slot_helper_matches_the_cluster_layout() {
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    assert!(redis_slot(low) <= 8191);
    assert!(redis_slot(high) >= 8192);
}
