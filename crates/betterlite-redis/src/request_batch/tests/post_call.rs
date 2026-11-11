use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::{advance, timeout};

use super::support::{cache, command_names};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::scope::{
    ACTIVE_REQUEST_BATCHES, active_post_call_redis_batch, active_request_redis_batch,
    flush_post_call_redis_batches, request_redis_batch_scope,
};

#[tokio::test]
async fn request_batch_post_call_writes_flush_separately() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Nil]));
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;

    let (_, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let request = ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .expect("a request scope");
        let reads = active_request_redis_batch(&cached).expect("a request batch");
        let writes = active_post_call_redis_batch(&cached).expect("a post-call batch");
        assert!(Arc::ptr_eq(&reads, &request.batch(&cached)));
        assert!(Arc::ptr_eq(
            &writes,
            &active_post_call_redis_batch(&cached).expect("the same post-call batch"),
        ));

        let read = request.batch(&cached).mget(&["a".to_owned()]);
        writes.increment("counter", 1.5, Some(10));
        read.await.expect("the request read");
        assert_eq!(
            writes.pending(),
            1,
            "post-call writes do not ride the read flush"
        );
        assert_eq!(command_names(&server), ["MGET"]);

        flush_post_call_redis_batches().await;
        assert_eq!(writes.pending(), 0);
        assert_eq!(writes.flushes(), 1);
        assert_eq!(command_names(&server), ["MGET", "INCRBYFLOAT", "EXPIRE"]);
    })
    .await;
}

#[tokio::test]
async fn request_batch_flush_all_flushes_pending_request_batches() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Nil]));
    let cached = cache(&server, Some("ns")).await;

    let (request, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {}).await;
    let batch = request.batch(&cached);
    batch.mget(&["a".to_owned()]);
    assert_eq!(batch.pending(), 1);

    request.flush_all().await;
    assert_eq!(batch.pending(), 0);
    assert_eq!(batch.flushes(), 1);
}

#[tokio::test(start_paused = true)]
async fn request_batch_post_call_deadline_flushes_the_post_call_writes() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;
    let deadline = Duration::from_secs(1);

    let (_, ()) = request_redis_batch_scope(deadline, async {
        let writes = active_post_call_redis_batch(&cached).expect("a post-call batch");
        writes.increment("counter", 1.5, None);
        yield_now().await;
        assert_eq!(writes.pending(), 1);
        assert!(command_names(&server).is_empty());

        advance(deadline).await;
        yield_now().await;
        assert_eq!(writes.pending(), 0);
        assert_eq!(writes.flushes(), 1);
        assert_eq!(command_names(&server), ["INCRBYFLOAT"]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn request_batch_early_flush_cancels_the_deadline_timer() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;
    let deadline = Duration::from_secs(1);

    let (_, ()) = request_redis_batch_scope(deadline, async {
        let writes = active_post_call_redis_batch(&cached).expect("a post-call batch");
        writes
            .increment("counter", 1.5, None)
            .await
            .expect("the first write");
        yield_now().await;
        advance(Duration::from_millis(500)).await;
        yield_now().await;
        flush_post_call_redis_batches().await;

        let later = active_post_call_redis_batch(&cached).expect("the same post-call batch");
        assert!(Arc::ptr_eq(&writes, &later));
        later.increment("second", 2.5, None);
        advance(Duration::from_millis(700)).await;
        yield_now().await;
        assert_eq!(
            later.pending(),
            1,
            "the canceled timer does not flush the later write"
        );
        assert_eq!(command_names(&server), ["INCRBYFLOAT"]);

        advance(Duration::from_millis(400)).await;
        yield_now().await;
        assert_eq!(later.pending(), 0, "the re-armed timer flushes it");
        assert_eq!(command_names(&server), ["INCRBYFLOAT", "INCRBYFLOAT"]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn request_batch_flush_all_cancels_an_idle_deadline() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    server.script("INCRBYFLOAT", Reply::Bulk("2.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;
    let deadline = Duration::from_secs(1);

    let (_, ()) = request_redis_batch_scope(deadline, async {
        let request = ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .expect("a request scope");
        let writes = active_post_call_redis_batch(&cached).expect("a post-call batch");
        writes
            .increment("counter", 1.5, None)
            .await
            .expect("the first write");
        yield_now().await;
        advance(Duration::from_millis(500)).await;
        yield_now().await;
        request.flush_all().await;

        let later = active_post_call_redis_batch(&cached).expect("the same post-call batch");
        later.increment("second", 2.5, None);
        advance(Duration::from_millis(700)).await;
        yield_now().await;
        assert_eq!(
            later.pending(),
            1,
            "flush_all disarmed the timer with no post-call writes left"
        );
        assert_eq!(command_names(&server), ["INCRBYFLOAT"]);

        advance(Duration::from_millis(400)).await;
        yield_now().await;
        assert_eq!(later.pending(), 0);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn request_batch_flush_all_keeps_a_deadline_with_pending_writes() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;
    let deadline = Duration::from_secs(1);

    let (_, ()) = request_redis_batch_scope(deadline, async {
        let request = ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .expect("a request scope");
        let writes = active_post_call_redis_batch(&cached).expect("a post-call batch");
        writes.increment("counter", 1.5, None);
        yield_now().await;
        request.flush_all().await;
        assert_eq!(
            writes.pending(),
            1,
            "flush_all leaves post-call writes alone"
        );

        advance(deadline).await;
        yield_now().await;
        assert_eq!(writes.pending(), 0, "the armed deadline flushes them");
        assert_eq!(command_names(&server), ["INCRBYFLOAT"]);
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn request_batch_deadline_flush_survives_request_cancellation() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;
    let deadline = Duration::from_secs(1);

    let operation = request_redis_batch_scope(deadline, async {
        active_post_call_redis_batch(&cached)
            .expect("a post-call batch")
            .increment("counter", 1.5, None);
        pending::<()>().await;
    });
    let cancelled = timeout(Duration::from_millis(100), operation).await;
    assert!(cancelled.is_err(), "the request scope is cancelled");
    yield_now().await;
    assert!(command_names(&server).is_empty());

    advance(deadline).await;
    yield_now().await;
    assert_eq!(command_names(&server), ["INCRBYFLOAT"]);
}
