#![expect(
    clippy::disallowed_types,
    reason = "tokio's task_local macro keeps its slot in a thread-local RefCell; that is the runtime's storage, not a borrow-checked value of ours"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use tokio::task_local;
use tokio_util::task::TaskTracker;

use super::batches::RequestRedisBatches;
use crate::batch::RedisBatch;
use crate::redis_cache::RedisCache;

task_local! {
    static ACTIVE_REQUEST_BATCHES: Arc<RequestRedisBatches>;
}

pub async fn request_redis_batch_scope<F>(
    post_call_deadline: Duration,
    operation: F,
) -> (Arc<RequestRedisBatches>, F::Output)
where
    F: Future,
{
    if let Ok(outer) = ACTIVE_REQUEST_BATCHES.try_with(Arc::clone) {
        return (outer, operation.await);
    }
    let batches = Arc::new(RequestRedisBatches {
        post_call_deadline,
        batches: Mutex::new(IndexMap::new()),
        post_call: Mutex::new(IndexMap::new()),
        deadline: Mutex::new(None),
        prefetched: Mutex::new(BTreeMap::new()),
        follow_ups: TaskTracker::new(),
    });
    let output = ACTIVE_REQUEST_BATCHES
        .scope(Arc::clone(&batches), operation)
        .await;
    (batches, output)
}

#[must_use]
pub fn active_request_redis_batches() -> Option<Arc<RequestRedisBatches>> {
    if let Ok(active) = ACTIVE_REQUEST_BATCHES.try_with(Arc::clone) {
        return Some(active);
    }
    None
}

#[must_use]
pub fn active_request_redis_batch(redis_cache: &Arc<RedisCache>) -> Option<Arc<RedisBatch>> {
    if let Ok(batch) = ACTIVE_REQUEST_BATCHES.try_with(|active| active.batch(redis_cache)) {
        return Some(batch);
    }
    None
}

#[must_use]
pub fn active_post_call_redis_batch(redis_cache: &Arc<RedisCache>) -> Option<Arc<RedisBatch>> {
    if let Ok(batch) = ACTIVE_REQUEST_BATCHES.try_with(|active| active.post_call(redis_cache)) {
        return Some(batch);
    }
    None
}

pub async fn flush_post_call_redis_batches() {
    if let Some(batches) = active_request_redis_batches() {
        batches.flush_post_call().await;
    }
}
