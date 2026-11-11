use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::sleep;

use super::support::{CountingGate, cache, cache_with_gate, command_names};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::lock;
use crate::request_batch::registry::{self, Change, apply, drain_post_call_redis_batches};
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

#[tokio::test]
async fn request_batch_drain_flushes_open_requests_concurrently() {
    let first_server = FakeServer::start().await;
    first_server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let second_server = FakeServer::start().await;
    second_server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    let gate = Arc::new(CountingGate::new());
    let first_cache = cache_with_gate(&first_server, Some("ns"), 0.2, Arc::clone(&gate)).await;
    let second_cache = cache_with_gate(&second_server, Some("ns"), 0.2, Arc::clone(&gate)).await;
    let (first, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let (second, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    first.post_call(&first_cache).increment("first", 1.5, None);
    second
        .post_call(&second_cache)
        .increment("second", 2.5, None);
    gate.reset_peak();

    drain_post_call_redis_batches().await;
    assert_eq!(
        gate.peak(),
        2,
        "both backends' pipelines run at the same time"
    );
}

#[tokio::test]
async fn request_batch_flush_all_forgets_an_idle_request() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;

    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let writes = request.post_call(&cached);
    writes
        .increment("counter", 1.5, None)
        .await
        .expect("the post-call write");
    request.flush_all().await;
    let stored = Arc::downgrade(&request);
    drop(request);

    let held = lock(&registry::OPEN_POST_CALL)
        .iter()
        .any(|weak| weak.ptr_eq(&stored));
    assert!(!held, "flush_all drops an idle request from the registry");
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
    assert!(
        !lock(&registry::OPEN_POST_CALL)
            .iter()
            .any(|held| held.ptr_eq(&weak)),
        "drain drops the dead entry"
    );

    let (pruned, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    apply(&pruned, Change::Register);
    let pruned_weak = Arc::downgrade(&pruned);
    drop(pruned);
    let (next, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    apply(&next, Change::Register);
    assert!(
        !lock(&registry::OPEN_POST_CALL)
            .iter()
            .any(|held| held.ptr_eq(&pruned_weak)),
        "registering drops the dead entry"
    );
    drain_post_call_redis_batches().await;
}

#[tokio::test(start_paused = true)]
async fn request_batch_drain_waits_for_the_requests_spawned_tasks() {
    let server = FakeServer::start().await;
    let cached = cache(&server, Some("ns")).await;
    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let writes = request.post_call(&cached);
    assert_eq!(writes.pending(), 0, "a post-call batch with no writes");
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    request.follow_ups.spawn(async move {
        sleep(Duration::from_secs(1)).await;
        flag.store(true, Ordering::Relaxed);
    });

    drain_post_call_redis_batches().await;
    assert!(
        finished.load(Ordering::Relaxed),
        "drain waits for the request's spawned tasks"
    );
}
