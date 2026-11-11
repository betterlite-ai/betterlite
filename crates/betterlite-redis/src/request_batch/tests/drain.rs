use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use super::support::{cache, command_names};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::registry::{Change, apply, drain_post_call_redis_batches};
use crate::request_batch::scope::request_redis_batch_scope;

#[tokio::test]
async fn request_batch_drain_flushes_every_open_post_call_request() {
    let first_server = FakeServer::start().await;
    first_server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let second_server = FakeServer::start().await;
    second_server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    let first_cache = cache(&first_server, Some("ns")).await;
    let second_cache = cache(&second_server, Some("ns")).await;

    let (first, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let (second, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let first_writes = first.post_call(&first_cache);
    let second_writes = second.post_call(&second_cache);
    first_writes.increment("first", 1.5, None);
    second_writes.increment("second", 2.5, None);
    assert_eq!(first_writes.pending(), 1);
    assert_eq!(second_writes.pending(), 1);

    drain_post_call_redis_batches().await;
    assert_eq!(first_writes.pending(), 0);
    assert_eq!(second_writes.pending(), 0);
    assert_eq!(command_names(&first_server), ["INCRBYFLOAT"]);
    assert_eq!(command_names(&second_server), ["INCRBYFLOAT"]);
}

#[tokio::test]
async fn request_batch_flushing_one_request_keeps_the_others_registered() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;

    let (flushed, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let (waiting, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let flushed_writes = flushed.post_call(&cached);
    let waiting_writes = waiting.post_call(&cached);
    flushed_writes.increment("flushed", 1.5, None);
    waiting_writes.increment("waiting", 2.5, None);

    flushed.flush_post_call().await;
    assert_eq!(flushed_writes.pending(), 0);
    assert_eq!(waiting_writes.pending(), 1);

    drain_post_call_redis_batches().await;
    assert_eq!(
        waiting_writes.pending(),
        0,
        "drain still sees the unflushed request"
    );
}

#[tokio::test(start_paused = true)]
async fn request_batch_drain_flushes_open_requests_concurrently() {
    let first_server = FakeServer::start().await;
    let second_server = FakeServer::start().await;
    let first_cache = cache(&first_server, Some("ns")).await;
    let second_cache = cache(&second_server, Some("ns")).await;
    let (first, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let (second, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    first.post_call(&first_cache).increment("first", 1.5, None);
    second
        .post_call(&second_cache)
        .increment("second", 2.5, None);
    first_server.mute();
    second_server.mute();

    let drained = timeout(Duration::from_millis(1100), drain_post_call_redis_batches()).await;
    assert!(
        drained.is_ok(),
        "both backends time out together, not one after the other"
    );
}

#[tokio::test]
async fn request_batch_weak_registry_forgets_a_dropped_request() {
    let (dropped, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    apply(&dropped, Change::Register);
    let weak = Arc::downgrade(&dropped);
    drop(dropped);
    assert_eq!(
        weak.strong_count(),
        0,
        "a registered request stays only weakly held"
    );
    assert!(weak.upgrade().is_none());
    drain_post_call_redis_batches().await;
}
