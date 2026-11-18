#![cfg(test)]

use super::commit_then_invalidate;
use crate::budget_reset_policy::cache_invalidation::{
    CacheDeleteFuture, InvalidatedCache, ResetCache,
};
use std::future::{Ready, ready};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug, thiserror::Error)]
#[error("the commit failed")]
struct OrderFault;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    fn push(&self, event: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.to_owned());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone)]
struct EventCache {
    events: Events,
    fail_spend: bool,
}

impl EventCache {
    fn note(&self, sink: InvalidatedCache, keys: &[String]) {
        self.events.push(&format!(
            "{name} {count}",
            name = sink.as_str(),
            count = keys.len()
        ));
    }
}

impl ResetCache for EventCache {
    type Failure = OrderFault;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, OrderFault> {
        self.note(InvalidatedCache::SpendCounter, keys);
        settle(self.fail_spend)
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, OrderFault> {
        self.note(InvalidatedCache::UserApiKeyCache, keys);
        Box::pin(async { Ok(()) })
    }
}

fn settle(failed: bool) -> CacheDeleteFuture<'static, OrderFault> {
    Box::pin(async move { if failed { Err(OrderFault) } else { Ok(()) } })
}

fn event_cache(events: &Events, fail_spend: bool) -> EventCache {
    EventCache {
        events: events.clone(),
        fail_spend,
    }
}

fn committing(events: &Events) -> impl FnOnce() -> Ready<Result<(), OrderFault>> {
    let recorded = events.clone();
    move || {
        recorded.push("commit");
        ready(Ok::<(), OrderFault>(()))
    }
}

fn failing(events: &Events) -> impl FnOnce() -> Ready<Result<(), OrderFault>> {
    let recorded = events.clone();
    move || {
        recorded.push("commit");
        ready(Err(OrderFault))
    }
}

#[tokio::test]
async fn invalidation_order_commits_before_dropping_caches() {
    let events = Events::default();
    let cache = event_cache(&events, false);
    let result = commit_then_invalidate(
        &cache,
        committing(&events),
        &["spend:key:k1".to_owned()],
        &["k1".to_owned()],
    )
    .await;
    result.unwrap();
    assert_eq!(
        events.entries(),
        ["commit", "spend counter 1", "user_api_key_cache 1"]
    );
}

#[tokio::test]
async fn invalidation_order_drops_no_cache_when_the_commit_fails() {
    let events = Events::default();
    let cache = event_cache(&events, false);
    let result = commit_then_invalidate(
        &cache,
        failing(&events),
        &["spend:key:k1".to_owned()],
        &["k1".to_owned()],
    )
    .await;
    assert!(matches!(result, Err(OrderFault)));
    assert_eq!(events.entries(), ["commit"]);
}

#[tokio::test]
async fn invalidation_order_skips_empty_cache_lists() {
    let events = Events::default();
    let cache = event_cache(&events, true);
    let result = commit_then_invalidate(&cache, committing(&events), &[], &[]).await;
    result.unwrap();
    assert_eq!(events.entries(), ["commit"]);
}

#[tokio::test]
async fn invalidation_order_keeps_the_commit_when_a_cache_drop_fails() {
    let events = Events::default();
    let cache = event_cache(&events, true);
    let result = commit_then_invalidate(
        &cache,
        committing(&events),
        &["spend:key:k1".to_owned()],
        &["k1".to_owned()],
    )
    .await;
    result.unwrap();
    assert_eq!(
        events.entries(),
        ["commit", "spend counter 1", "user_api_key_cache 1"]
    );
}
