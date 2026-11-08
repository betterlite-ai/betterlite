use redis::Value;

use super::fixtures::{
    cluster_pair, cluster_settings, direct_settings, key_in_slot_range, store, transport,
};
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn async_mget_returns_values_in_input_order_with_nil_for_a_miss() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("alpha".to_owned()), Reply::Nil]),
    );
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let values = transport
        .async_mget(&["alpha".to_owned(), "missing".to_owned()])
        .await
        .expect("the async mget");
    assert_eq!(
        values,
        vec![Value::BulkString(b"alpha".to_vec()), Value::Nil]
    );
}

#[tokio::test]
async fn sync_mget_returns_values_in_input_order_with_nil_for_a_miss() {
    let server = FakeServer::start().await;
    server.script(
        "MGET",
        Reply::Array(vec![Reply::Nil, Reply::Bulk("beta".to_owned())]),
    );
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let values = transport
        .mget(&["missing".to_owned(), "beta".to_owned()])
        .expect("the sync mget");
    assert_eq!(
        values,
        vec![Value::Nil, Value::BulkString(b"beta".to_vec())]
    );
}

#[tokio::test]
async fn a_duplicate_key_returns_its_value_for_each_position() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::EchoArgs);
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    let values = transport
        .async_mget(&["alpha".to_owned(), "alpha".to_owned()])
        .await
        .expect("the async mget");
    assert_eq!(
        values,
        vec![
            Value::BulkString(b"alpha".to_vec()),
            Value::BulkString(b"alpha".to_vec())
        ]
    );
    assert_eq!(server.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn a_cross_slot_mget_returns_values_in_input_order_across_nodes() {
    let (first, second) = cluster_pair().await;
    first.script("MGET", Reply::EchoArgs);
    second.script("MGET", Reply::EchoArgs);
    let store = store();
    let settings = cluster_settings(&[(first.host(), first.port())]);
    let transport = transport(settings, RedisCacheFamily::Cluster, &store, None);
    let low = key_in_slot_range(0, 8191).to_owned();
    let high = key_in_slot_range(8192, 16383).to_owned();
    let values = transport
        .async_mget(&[low.clone(), high.clone()])
        .await
        .expect("the cross-slot mget");
    assert_eq!(
        values,
        vec![
            Value::BulkString(low.into_bytes()),
            Value::BulkString(high.into_bytes())
        ]
    );
    assert_eq!(first.commands_named("MGET").len(), 1);
    assert_eq!(second.commands_named("MGET").len(), 1);
}

#[tokio::test]
async fn empty_keys_return_an_empty_list_and_send_nothing() {
    let server = FakeServer::start().await;
    let store = store();
    let settings = direct_settings(&server.host(), server.port());
    let transport = transport(settings, RedisCacheFamily::Standalone, &store, None);
    assert!(
        transport
            .async_mget(&[])
            .await
            .expect("the empty async mget")
            .is_empty()
    );
    assert!(transport.mget(&[]).expect("the empty sync mget").is_empty());
    assert!(server.commands_named("MGET").is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn sync_mget_returns_values_in_input_order_across_two_cluster_nodes() {
    let (first, second) = cluster_pair().await;
    first.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("first-value".to_owned()), Reply::Nil]),
    );
    second.script(
        "MGET",
        Reply::Array(vec![Reply::Bulk("second-value".to_owned())]),
    );
    let store = store();
    let settings = cluster_settings(&[(first.host(), first.port())]);
    let transport = transport(settings, RedisCacheFamily::Cluster, &store, None);
    let low = key_in_slot_range(0, 8191).to_owned();
    let missing = format!("{{{low}}}missing");
    let high = key_in_slot_range(8192, 16383).to_owned();
    let values = transport
        .mget(&[low, high, missing])
        .expect("the sync cross-slot mget");
    assert_eq!(
        values,
        vec![
            Value::BulkString(b"first-value".to_vec()),
            Value::BulkString(b"second-value".to_vec()),
            Value::Nil
        ]
    );
    assert_eq!(first.commands_named("MGET").len(), 1);
    assert_eq!(second.commands_named("MGET").len(), 1);
}
