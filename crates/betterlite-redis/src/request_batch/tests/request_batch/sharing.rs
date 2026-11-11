use std::sync::Arc;

use super::support::{cache, command_names};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::lock;
use crate::request_batch::scope::{
    ACTIVE_REQUEST_BATCHES, active_post_call_redis_batch, active_request_redis_batch,
    flush_post_call_redis_batches, request_redis_batch_scope,
};

#[tokio::test]
async fn request_batch_shares_one_batch_per_backend() {
    let first = FakeServer::start().await;
    let second = FakeServer::start().await;
    let cache_a = cache(&first, Some("ns")).await;
    let cache_b = cache(&first, Some("ns")).await;
    let cache_c = cache(&first, Some("other")).await;
    let cache_d = cache(&second, Some("ns")).await;

    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let batch_a = request.batch(&cache_a);
    let batch_b = request.batch(&cache_b);
    let batch_c = request.batch(&cache_c);
    let batch_d = request.batch(&cache_d);

    assert!(
        Arc::ptr_eq(&batch_a, &batch_b),
        "same settings and namespace"
    );
    assert!(!Arc::ptr_eq(&batch_a, &batch_c), "a different namespace");
    assert!(!Arc::ptr_eq(&batch_a, &batch_d), "different settings");
    assert_eq!(lock(&request.batches).len(), 3);
}

#[tokio::test]
async fn request_batch_awaiting_one_operation_flushes_the_shared_batch() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Bulk("1".to_owned())]));
    server.script("MGET", Reply::Array(vec![Reply::Bulk("2".to_owned())]));
    let other_server = FakeServer::start().await;
    other_server.script("MGET", Reply::Array(vec![Reply::Nil]));
    let cache_a = cache(&server, Some("ns")).await;
    let cache_b = cache(&server, Some("ns")).await;
    let cache_c = cache(&other_server, Some("ns")).await;

    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let shared = request.batch(&cache_a);
    let second = request.batch(&cache_b);
    let elsewhere = request.batch(&cache_c);
    let first_read = shared.mget(&["a".to_owned()]);
    let second_read = second.mget(&["b".to_owned()]);
    let other_read = elsewhere.mget(&["c".to_owned()]);
    assert_eq!(shared.pending(), 2);
    assert_eq!(elsewhere.pending(), 1);

    let values = first_read.await.expect("the shared read");
    assert_eq!(values.len(), 1);
    assert!(values.contains_key("a"));
    assert_eq!(shared.flushes(), 1);
    assert_eq!(command_names(&server), ["MGET", "MGET"]);
    assert_eq!(elsewhere.flushes(), 0, "another backend flushes on its own");
    assert!(command_names(&other_server).is_empty());

    second_read.await.expect("the already settled double");
    assert_eq!(shared.flushes(), 1, "a settled read does not flush again");
    other_read.await.expect("the other backend read");
    assert_eq!(elsewhere.flushes(), 1);
    assert_eq!(command_names(&other_server), ["MGET"]);
}

#[tokio::test]
async fn request_batch_accessors_are_empty_outside_a_scope() {
    let server = FakeServer::start().await;
    let cached = cache(&server, Some("ns")).await;

    assert!(ACTIVE_REQUEST_BATCHES.try_with(Arc::clone).is_err());
    assert!(active_request_redis_batch(&cached).is_none());
    assert!(active_post_call_redis_batch(&cached).is_none());
    flush_post_call_redis_batches().await;
}
