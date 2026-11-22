use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::batch::result::SettledHook;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, request_redis_batch_scope,
};

use crate::spend_writes::layers::CounterLayers as _;

use super::refusing::refusing_redis_cache;
use super::support::{FakePostCall, KEY, OTHER_KEY, dual_cache, fixture, increment};

#[tokio::test]
async fn defer_queues_every_increment_on_the_post_call_batch() {
    let fx = fixture(true);
    let post = Arc::new(FakePostCall::new(60, fx.journal.clone()));
    fx.layers.serve_post_call(Arc::clone(&post));
    let deferred = fx
        .writes
        .defer_spend_counter_increments(&[increment(KEY, "2"), increment(OTHER_KEY, "3")]);
    assert!(deferred);
    assert_eq!(
        post.queued(),
        vec![
            (KEY.to_owned(), 2.0_f64, 60_i64),
            (OTHER_KEY.to_owned(), 3.0_f64, 60_i64),
        ]
    );
    let expected = vec![
        "post_call".to_owned(),
        format!("queue:{KEY}"),
        format!("queue:{OTHER_KEY}"),
    ];
    assert_eq!(fx.journal.events(), expected);
}

#[tokio::test]
async fn defer_without_a_post_call_batch_returns_false() {
    let fx = fixture(true);
    let deferred = fx
        .writes
        .defer_spend_counter_increments(&[increment(KEY, "2")]);
    assert!(!deferred);
    assert!(fx.journal.records("post_call"));
    assert_eq!(fx.journal.count("queue:"), 0);
}

#[tokio::test]
async fn defer_does_not_defer_an_empty_list() {
    let fx = fixture(true);
    let post = Arc::new(FakePostCall::new(60, fx.journal.clone()));
    fx.layers.serve_post_call(Arc::clone(&post));
    let deferred = fx.writes.defer_spend_counter_increments(&[]);
    assert!(!deferred);
    assert!(post.queued().is_empty());
    assert!(fx.journal.events().is_empty());
}

#[tokio::test]
async fn defer_returns_false_without_redis() {
    let fx = fixture(false);
    let post = Arc::new(FakePostCall::new(60, fx.journal.clone()));
    fx.layers.serve_post_call(Arc::clone(&post));
    let deferred = fx
        .writes
        .defer_spend_counter_increments(&[increment(KEY, "2")]);
    assert!(!deferred);
    assert!(post.queued().is_empty());
}

#[tokio::test]
async fn the_real_post_call_batch_queues_an_increment() {
    let redis = refusing_redis_cache().await;
    let cache = dual_cache(Some(Arc::clone(&redis)));
    let (_batches, queued) = request_redis_batch_scope(Duration::from_secs(1), async {
        let batch =
            active_post_call_redis_batch(&redis).expect("the scope serves a post-call batch");
        let post = cache
            .post_call()
            .expect("the real tier serves its post-call batch");
        assert_eq!(post.ttl, 60);
        let hook: SettledHook<f64> = Arc::new(|_settled| None);
        post.batch.increment(KEY, 1.5, post.ttl, hook);
        batch.pending()
    })
    .await;
    assert_eq!(queued, 1_usize);
}

#[tokio::test]
async fn the_real_tier_has_no_post_call_batch_outside_a_request() {
    let cache = dual_cache(Some(refusing_redis_cache().await));
    assert!(cache.post_call().is_none());
}

#[tokio::test]
async fn the_real_tier_has_no_post_call_batch_without_redis() {
    let cache = dual_cache(None);
    assert!(cache.post_call().is_none());
}
