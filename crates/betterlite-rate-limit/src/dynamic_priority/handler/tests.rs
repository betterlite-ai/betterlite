#![cfg(test)]

use std::sync::Arc;

use super::DynamicRateLimitHandler;
use crate::dynamic_priority::cache::SaturationCache;
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::dynamic_priority::tests::support::{FixedClock, RecordingCache};
use crate::limiter_records::LimiterClock;

struct FakeRouter;

#[test]
fn the_constructor_shares_the_cache_with_the_v3_limiter() {
    let cache: Arc<dyn SaturationCache> = Arc::new(RecordingCache::default());
    let clock: Arc<dyn LimiterClock> = Arc::new(FixedClock);
    let handler = DynamicRateLimitHandler::<FakeRouter>::new(
        Arc::clone(&cache),
        Some(Arc::clone(&clock)),
        PriorityPolicy::default(),
        30,
    );
    assert!(Arc::ptr_eq(&handler.internal_usage_cache, &cache));
    assert!(Arc::ptr_eq(
        &handler.v3_limiter.internal_usage_cache,
        &cache
    ));
    let stored = handler
        .v3_limiter
        .time_provider
        .as_ref()
        .expect("a time provider");
    assert!(Arc::ptr_eq(stored, &clock));
    assert_eq!(handler.v3_limiter.window_size, 30);
    assert!(handler.llm_router.is_none());
}

#[test]
fn an_absent_time_provider_stays_absent() {
    let handler = DynamicRateLimitHandler::<FakeRouter>::new(
        Arc::new(RecordingCache::default()),
        None,
        PriorityPolicy::default(),
        60,
    );
    assert!(handler.v3_limiter.time_provider.is_none());
}

#[test]
fn update_variables_stores_the_router() {
    let mut handler = DynamicRateLimitHandler::<FakeRouter>::new(
        Arc::new(RecordingCache::default()),
        None,
        PriorityPolicy::default(),
        60,
    );
    let router = Arc::new(FakeRouter);
    handler.update_variables(Arc::clone(&router));
    let stored = handler.llm_router.as_ref().expect("a router");
    assert!(Arc::ptr_eq(stored, &router));
}
