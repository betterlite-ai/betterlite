use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::SecretString;

use super::fixtures::pubsub_server::{ClusterSlots, PubSubServer};
use super::fixtures::{cluster_record, cluster_settings, direct_settings, store, transport};
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::pubsub::cluster_node_connection;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::scalars::PassthroughValue;

fn topology(server: &PubSubServer) -> ClusterSlots {
    vec![(0, 16383, vec![server.node()])]
}

#[test]
fn cluster_node_connection_picks_the_first_startup_node() {
    let record = cluster_record(&[
        ("first.internal".to_owned(), 7000),
        ("second.internal".to_owned(), 7001),
    ]);
    let fields = cluster_node_connection(&record).expect("the first node");
    assert_eq!(fields.host.as_deref(), Some("first.internal"));
    assert_eq!(fields.port, Some(7000));
}

#[test]
fn cluster_node_connection_keeps_auth_tls_and_timeouts() {
    let mut record = cluster_record(&[("first.internal".to_owned(), 7000)]);
    record.username = Some("app".to_owned());
    record.password = Some(SecretString::from("secret".to_owned()));
    record.ssl = Some(true);
    record.ssl_password = Some(SecretString::from("ssl-secret".to_owned()));
    record.socket_timeout = Some(2.5_f64);
    record.socket_connect_timeout = Some(1.5_f64);
    record.socket_keepalive = Some(true);
    record.max_connections = Some(25);
    record.health_check_interval = Some(10);
    record.credential_provider = true;
    record.passthrough = BTreeMap::from([
        (
            "read_from_replicas".to_owned(),
            PassthroughValue::Bool(true),
        ),
        ("decode_responses".to_owned(), PassthroughValue::Bool(true)),
    ]);
    let fields = cluster_node_connection(&record).expect("the node connection");
    assert_eq!(fields.username.as_deref(), Some("app"));
    assert!(fields.password.is_some());
    assert_eq!(fields.ssl, Some(true));
    assert!(fields.ssl_password.is_some());
    assert_eq!(fields.socket_timeout, Some(2.5_f64));
    assert_eq!(fields.socket_connect_timeout, Some(1.5_f64));
    assert_eq!(fields.socket_keepalive, Some(true));
    assert_eq!(fields.max_connections, Some(25));
    assert_eq!(fields.health_check_interval, Some(10));
    assert!(fields.credential_provider);
    assert!(!fields.passthrough.contains_key("read_from_replicas"));
    assert!(fields.passthrough.contains_key("decode_responses"));
}

#[test]
fn cluster_node_connection_with_no_nodes_is_the_litellm_error() {
    let record = cluster_record(&[]);
    let error = cluster_node_connection(&record).expect_err("a cluster without nodes");
    assert!(matches!(error, ClientReuseError::NoPubSubNode));
    assert_eq!(
        error.to_string(),
        "cannot derive a pub/sub client: redis cluster has no default node and no startup nodes"
    );
}

#[tokio::test]
async fn a_plain_client_serves_pubsub_as_it_is() {
    let server = PubSubServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let first = transport.pubsub_client().await.expect("a pubsub client");
    assert_eq!(
        first.publish("channel", "hello").await.expect("a publish"),
        0
    );
    assert_eq!(server.connections(), 1);
    let second = transport.pubsub_client().await.expect("the same client");
    assert_eq!(
        second.publish("channel", "hello").await.expect("a publish"),
        0
    );
    assert_eq!(server.connections(), 1);
    let publishes = server.commands_named("PUBLISH");
    assert_eq!(publishes.len(), 2);
    assert_eq!(publishes[0].name, "PUBLISH");
    assert_eq!(publishes[0].args, ["PUBLISH", "channel", "hello"]);
}

#[tokio::test]
async fn a_cluster_client_shares_one_node_client_under_the_pubsub_key() {
    let server = PubSubServer::start().await;
    server.set_cluster_slots(topology(&server));
    let store = store();
    let settings = cluster_settings(&[server.node()]);
    let transport = transport(settings, RedisCacheFamily::Cluster, &store, None);
    let first = transport
        .pubsub_client()
        .await
        .expect("the first pubsub client");
    first.publish("channel", "one").await.expect("a publish");
    let after_first = server.connections();
    let second = transport
        .pubsub_client()
        .await
        .expect("the shared pubsub client");
    second.publish("channel", "two").await.expect("a publish");
    assert_eq!(server.connections(), after_first);
    transport.close_pubsub_client().await;
    let reopened = transport
        .pubsub_client()
        .await
        .expect("the reopened pubsub client");
    reopened
        .publish("channel", "three")
        .await
        .expect("a publish");
    assert!(
        server.connections() > after_first,
        "close_pubsub_client removed the shared node client"
    );
    assert_eq!(server.commands_named("PUBLISH").len(), 3);
}

#[tokio::test]
async fn a_standalone_family_with_startup_nodes_derives_a_node_client() {
    let server = PubSubServer::start().await;
    server.set_cluster_slots(topology(&server));
    let store: Arc<RedisClientStore> = store();
    let settings = cluster_settings(&[server.node()]);
    let first = transport(settings.clone(), RedisCacheFamily::Standalone, &store, None);
    let second = transport(settings, RedisCacheFamily::Standalone, &store, None);
    first
        .async_client()
        .await
        .expect("the stored cluster client");
    let after_cluster = server.connections();
    second
        .async_client()
        .await
        .expect("the shared cluster client");
    assert_eq!(server.connections(), after_cluster);
    let first_pubsub = first.pubsub_client().await.expect("a pubsub client");
    first_pubsub
        .publish("channel", "one")
        .await
        .expect("a publish");
    let after_pubsub = server.connections();
    let second_pubsub = second
        .pubsub_client()
        .await
        .expect("the shared pubsub client");
    second_pubsub
        .publish("channel", "two")
        .await
        .expect("a publish");
    assert_eq!(server.connections(), after_pubsub);
    assert!(after_pubsub > after_cluster);
    assert_eq!(server.commands_named("PUBLISH").len(), 2);
}
