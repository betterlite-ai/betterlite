use std::time::Duration;

use redis::cmd;

use super::fake_server::FakeServer;
use super::fixtures::{
    cluster_pair, cluster_record, credential, direct_fields, factory, key_in_slot_range,
    sentinel_record, source,
};
use crate::redis_client_factories::RedisClient;
use crate::redis_live_client::LiveTarget;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_endpoint_returns_a_usable_standalone_client() {
    let server = FakeServer::start().await;
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Direct(direct_fields(
            &server.host(),
            server.port(),
        )))
        .expect("an endpoint");
    let LiveTarget::Standalone(client) = endpoint.target else {
        panic!("a direct record builds a standalone target");
    };
    let mut connection = client.get_connection().expect("a sync connection");
    let reply: String = cmd("PING").query(&mut connection).expect("a reply");
    assert_eq!(reply, "PONG");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blocking_endpoint_returns_a_usable_cluster_client() {
    let (first, second) = cluster_pair().await;
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Cluster(cluster_record(&[&first])))
        .expect("an endpoint");
    let LiveTarget::Cluster(client) = endpoint.target else {
        panic!("a cluster record builds a cluster target");
    };
    let mut connection = client.get_connection().expect("a sync cluster connection");
    let key = key_in_slot_range(8192, 16383);
    let reply: String = cmd("SET")
        .arg(key)
        .arg("1")
        .query(&mut connection)
        .expect("a write");
    assert_eq!(reply, "OK");
    assert_eq!(second.commands_named("SET").len(), 1);
}

#[test]
fn the_blocking_endpoint_carries_the_record_timeouts() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.socket_timeout = Some(0.25_f64);
    fields.socket_connect_timeout = Some(0.5_f64);
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Direct(fields))
        .expect("an endpoint");
    assert_eq!(endpoint.response_timeout, Duration::from_millis(250));
    assert_eq!(endpoint.connect_timeout, Duration::from_millis(500));
}

#[test]
fn each_blocking_endpoint_call_fetches_a_fresh_credential() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.credential_provider = true;
    let credential_source = source(vec![credential(None, "tok-1"), credential(None, "tok-2")]);
    let factory = factory(Some(credential_source));
    let first = factory
        .blocking_endpoint(&RedisClient::Direct(fields.clone()))
        .expect("a first endpoint");
    let second = factory
        .blocking_endpoint(&RedisClient::Direct(fields))
        .expect("a second endpoint");
    let LiveTarget::Standalone(first_client) = first.target else {
        panic!("a standalone target");
    };
    let LiveTarget::Standalone(second_client) = second.target else {
        panic!("a standalone target");
    };
    assert_eq!(
        first_client
            .get_connection_info()
            .redis_settings()
            .password(),
        Some("tok-1")
    );
    assert_eq!(
        second_client
            .get_connection_info()
            .redis_settings()
            .password(),
        Some("tok-2")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sentinel_endpoint_resolves_the_master_as_standalone() {
    let master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor.set_sentinel_master(master.host(), master.port());
    let record = sentinel_record(&[monitor.addr()], "mymaster");
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Sentinel(record))
        .expect("an endpoint");
    let LiveTarget::Standalone(client) = endpoint.target else {
        panic!("the resolved master is a standalone target");
    };
    assert_eq!(
        client.get_connection_info().addr().to_string(),
        format!("{}:{}", master.host(), master.port())
    );
    let mut connection = client.get_connection().expect("a sync connection");
    let reply: String = cmd("PING").query(&mut connection).expect("a reply");
    assert_eq!(reply, "PONG");
}
