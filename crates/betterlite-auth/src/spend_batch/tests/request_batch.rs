use betterlite_redis::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use betterlite_redis::request_batch::scope::{
    active_request_redis_batch, request_redis_batch_scope,
};

use crate::spend_batch::batch::SpendCounterValue;
use crate::spend_batch::scope::SpendBatchContext;

use super::support::{FakeRedis, Reply, allow_cache, decimal, keys};

#[tokio::test]
async fn the_flush_hook_puts_counters_on_the_request_pipeline() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let context = SpendBatchContext::default();
        let _scope = context.enter(Some(&cache), &keys(&["spend:key:b", "spend:key:a"]));
        let request = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned())]));
        server.script_mget(Reply::Array(vec![
            Reply::Bulk("2".to_owned()),
            Reply::Bulk("3".to_owned()),
        ]));
        request
            .mget(&["auth:key".to_owned()])
            .await
            .expect("the prefetch read");
        assert_eq!(
            server.mget_keys(),
            [
                vec!["auth:key".to_owned()],
                vec!["spend:key:a".to_owned(), "spend:key:b".to_owned()],
            ]
        );
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            Some(SpendCounterValue::Amount(decimal("2")))
        );
        assert_eq!(
            context
                .read_spend_counter("spend:key:b")
                .await
                .expect("the read succeeds"),
            Some(SpendCounterValue::Amount(decimal("3")))
        );
        assert_eq!(server.mget_keys().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_read_drives_its_own_flush() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let context = SpendBatchContext::default();
        let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("5".to_owned())]));
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            Some(SpendCounterValue::Amount(decimal("5")))
        );
        assert_eq!(server.mget_keys(), [vec!["spend:key:a".to_owned()]]);
    })
    .await;
}

#[tokio::test]
async fn a_closed_batch_adds_nothing_to_the_pipeline() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let context = SpendBatchContext::default();
        let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
        context.release_spend_counter_batch();
        let request = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        request
            .mget(&["auth:key".to_owned()])
            .await
            .expect("the prefetch read");
        assert_eq!(server.mget_keys(), [vec!["auth:key".to_owned()]]);
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            None
        );
    })
    .await;
}

#[tokio::test]
async fn a_failed_request_result_falls_back_without_a_new_mget() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let context = SpendBatchContext::default();
        let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
        let request = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Error("ERR mget failed".to_owned()));
        server.script_mget(Reply::Error("ERR mget failed".to_owned()));
        let failed = request.mget(&["auth:key".to_owned()]).await;
        assert!(failed.is_err(), "the pipeline reports the failure");
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            None
        );
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            None
        );
        assert_eq!(server.mget_keys().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn a_late_bind_joins_the_next_flush() {
    let server = FakeRedis::start().await;
    let (_batches, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let cache = allow_cache(&server).await;
        let context = SpendBatchContext::default();
        let _scope = context.enter(Some(&cache), &keys(&["spend:key:a"]));
        let request = active_request_redis_batch(&cache).expect("the request batch");
        server.script_mget(Reply::Array(vec![Reply::Bulk("1".to_owned())]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("2".to_owned())]));
        request
            .mget(&["auth:key".to_owned()])
            .await
            .expect("the first prefetch read");
        assert_eq!(
            context
                .read_spend_counter("spend:key:a")
                .await
                .expect("the read succeeds"),
            Some(SpendCounterValue::Amount(decimal("2")))
        );
        context.bind_spend_counter_keys(&keys(&["spend:key:b"]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("3".to_owned())]));
        server.script_mget(Reply::Array(vec![Reply::Bulk("4".to_owned())]));
        request
            .mget(&["auth:key-2".to_owned()])
            .await
            .expect("the second prefetch read");
        assert_eq!(
            context
                .read_spend_counter("spend:key:b")
                .await
                .expect("the read succeeds"),
            Some(SpendCounterValue::Amount(decimal("4")))
        );
        assert_eq!(
            server.mget_keys(),
            [
                vec!["auth:key".to_owned()],
                vec!["spend:key:a".to_owned()],
                vec!["auth:key-2".to_owned()],
                vec!["spend:key:b".to_owned()],
            ]
        );
        server.script_mget(Reply::Array(vec![Reply::Nil]));
        request
            .mget(&["auth:key-3".to_owned()])
            .await
            .expect("the third prefetch read");
        assert_eq!(server.mget_keys().len(), 5);
    })
    .await;
}
