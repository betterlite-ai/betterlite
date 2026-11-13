use std::sync::Arc;

use tokio::task::JoinSet;

use super::support::cache;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::lock;
use crate::request_batch::scope::{
    ACTIVE_REQUEST_BATCHES, active_post_call_redis_batch, active_request_redis_batch,
    flush_post_call_redis_batches, request_redis_batch_scope,
};

#[tokio::test]
async fn request_batch_spawned_task_can_join_the_request_scope() {
    let server = FakeServer::start().await;
    server.script("INCRBYFLOAT", Reply::Bulk("1.5".to_owned()));
    let cached = cache(&server, Some("ns")).await;

    let (_, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let request = ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .expect("a request scope");
        let writes = active_post_call_redis_batch(&cached).expect("the request's post-call batch");
        let carried = Arc::clone(&request);
        let expected = Arc::clone(&writes);
        let target = Arc::clone(&cached);
        let mut tasks = JoinSet::new();
        tasks.spawn(ACTIVE_REQUEST_BATCHES.scope(carried, async move {
            let joined = active_post_call_redis_batch(&target)
                .expect("the carried request's post-call batch");
            assert!(Arc::ptr_eq(&joined, &expected));
            joined.increment("counter", 1.5, None);
            flush_post_call_redis_batches().await;
        }));
        tasks
            .join_next()
            .await
            .expect("the spawned write")
            .expect("the spawned write panicked");
        assert_eq!(
            writes.pending(),
            0,
            "the task flushed the request's own post-call batch"
        );
        assert_eq!(writes.flushes(), 1);
    })
    .await;
}

#[tokio::test]
async fn request_batch_nested_scopes_join_the_outer_batches() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Nil]));
    let cached = cache(&server, Some("ns")).await;

    let (outer, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let outer_request = ACTIVE_REQUEST_BATCHES
            .try_with(Arc::clone)
            .expect("the outer scope");
        let outer_batch = active_request_redis_batch(&cached).expect("the outer batch");

        let (nested, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
            let inner_request = ACTIVE_REQUEST_BATCHES
                .try_with(Arc::clone)
                .expect("the outer scope inside");
            assert!(Arc::ptr_eq(&outer_request, &inner_request));
            let inner_batch = active_request_redis_batch(&cached).expect("the outer batch inside");
            assert!(Arc::ptr_eq(&outer_batch, &inner_batch));
            inner_batch
                .mget(&["a".to_owned()])
                .await
                .expect("the nested read");
        })
        .await;

        assert!(Arc::ptr_eq(&outer_request, &nested));
        assert!(Arc::ptr_eq(
            &outer_batch,
            &active_request_redis_batch(&cached).expect("the outer batch after")
        ));
        assert_eq!(
            ACTIVE_REQUEST_BATCHES
                .try_with(Arc::clone)
                .ok()
                .map(|request| lock(&request.batches).len()),
            Some(1)
        );
    })
    .await;

    assert_eq!(lock(&outer.batches).len(), 1);
}
