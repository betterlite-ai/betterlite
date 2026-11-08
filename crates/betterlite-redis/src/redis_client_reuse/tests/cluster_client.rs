use super::fixtures::{cluster_settings, direct_settings, store, transport};
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{ClusterSlots, FakeServer};

fn topology(server: &FakeServer) -> ClusterSlots {
    vec![(0, 16383, vec![(server.host(), server.port())])]
}

fn cluster_server(server: &FakeServer) -> Vec<(String, u16)> {
    vec![(server.host(), server.port())]
}

#[tokio::test]
async fn a_cluster_transport_holds_one_client_per_instance() {
    let server = FakeServer::start().await;
    server.set_cluster_slots(topology(&server));
    let store = store();
    let settings = cluster_settings(&cluster_server(&server));
    let transport = transport(settings, RedisCacheFamily::Cluster, &store, None);
    transport.async_client().await.expect("the first client");
    let after_first = server.connections();
    transport.async_client().await.expect("the held client");
    assert_eq!(server.connections(), after_first);
}

#[tokio::test]
async fn two_cluster_transports_open_two_clients() {
    let server = FakeServer::start().await;
    server.set_cluster_slots(topology(&server));
    let store = store();
    let settings = cluster_settings(&cluster_server(&server));
    let first = transport(settings.clone(), RedisCacheFamily::Cluster, &store, None);
    let second = transport(settings, RedisCacheFamily::Cluster, &store, None);
    first.async_client().await.expect("the first client");
    let after_first = server.connections();
    second.async_client().await.expect("the second client");
    assert!(
        server.connections() > after_first,
        "the second instance opens its own client"
    );
}

#[tokio::test]
async fn a_cluster_transport_leaves_the_store_empty() {
    let server = FakeServer::start().await;
    server.set_cluster_slots(topology(&server));
    let store = store();
    let settings = cluster_settings(&cluster_server(&server));
    let held = transport(settings.clone(), RedisCacheFamily::Cluster, &store, None);
    held.async_client().await.expect("the held client");
    let after_held = server.connections();
    let stored = transport(settings, RedisCacheFamily::Standalone, &store, None);
    stored.async_client().await.expect("a fresh stored client");
    assert!(
        server.connections() > after_held,
        "the cluster transport stored nothing under the cache key"
    );
}

#[tokio::test]
async fn a_cluster_transport_does_not_hold_a_plain_client() {
    let server = FakeServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Cluster, &store, None);
    transport.async_client().await.expect("the first client");
    let after_first = server.connections();
    transport.async_client().await.expect("the next client");
    assert!(
        server.connections() > after_first,
        "a non-Cluster client is opened on every call"
    );
}
