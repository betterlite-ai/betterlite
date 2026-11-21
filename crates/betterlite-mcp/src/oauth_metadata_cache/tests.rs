#![cfg(test)]

use std::time::Duration;

use betterlite_db::mcp_stored_contracts::JsonValue;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::{CacheKey, MAX_SIZE, NEGATIVE_TTL_SECONDS, OAuthMetadataCache, TTL_SECONDS};

fn key(server_id: &str, resource_url: &str) -> CacheKey {
    CacheKey {
        server_id: server_id.to_owned(),
        resource_url: resource_url.to_owned(),
    }
}

fn url(index: usize) -> String {
    format!("https://idp.test/{index}")
}

fn payload(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

fn entry_count(cache: &OAuthMetadataCache) -> usize {
    cache.inner.lock().unwrap().entries.len()
}

fn holds_lock(cache: &OAuthMetadataCache, key: &CacheKey) -> bool {
    cache.inner.lock().unwrap().locks.contains_key(key)
}

fn fetcher_count(cache: &OAuthMetadataCache, key: &CacheKey) -> usize {
    cache
        .inner
        .lock()
        .unwrap()
        .fetchers
        .get(key)
        .copied()
        .unwrap_or(0)
}

#[tokio::test]
async fn lookup_hits_only_while_the_entry_is_unexpired() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let now = Instant::now();
    assert!(cache.store(&key, 0, Some(payload("metadata")), TTL_SECONDS, now));
    assert_eq!(cache.lookup(&key, now), Some(Some(payload("metadata"))));
    assert_eq!(
        cache.lookup(&key, now + Duration::from_secs(299)),
        Some(Some(payload("metadata")))
    );
    assert_eq!(
        cache.lookup(&key, now + Duration::from_secs(TTL_SECONDS)),
        None
    );
    assert_eq!(entry_count(&cache), 1);
}

#[tokio::test]
async fn lookup_returns_a_negative_entry_as_a_hit() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let now = Instant::now();
    assert!(cache.store(&key, 0, None, NEGATIVE_TTL_SECONDS, now));
    assert_eq!(cache.lookup(&key, now), Some(None));
    assert_eq!(
        cache.lookup(&key, now + Duration::from_secs(NEGATIVE_TTL_SECONDS)),
        None
    );
    assert_eq!(entry_count(&cache), 1);
}

#[tokio::test]
async fn prune_removes_expired_entries_only() {
    let cache = OAuthMetadataCache::default();
    let now = Instant::now();
    let short = key("srv", "https://idp.test/short");
    let long = key("srv", "https://idp.test/long");
    assert!(cache.store(&short, 0, Some(payload("short")), 10, now));
    assert!(cache.store(&long, 0, Some(payload("long")), TTL_SECONDS, now));
    cache.prune(now + Duration::from_secs(9));
    assert_eq!(entry_count(&cache), 2);
    cache.prune(now + Duration::from_secs(10));
    assert_eq!(entry_count(&cache), 1);
    assert_eq!(cache.lookup(&short, now), None);
    assert_eq!(cache.lookup(&long, now), Some(Some(payload("long"))));
}

#[tokio::test]
async fn prune_evicts_the_earliest_expiring_entries_and_ties_by_insert_order() {
    let cache = OAuthMetadataCache::default();
    let now = Instant::now();
    let early = key("srv", &url(0));
    let tied = key("srv", &url(1));
    assert!(cache.store(&early, 0, Some(payload("early")), NEGATIVE_TTL_SECONDS, now));
    assert!(cache.store(&tied, 0, Some(payload("tied")), NEGATIVE_TTL_SECONDS, now));
    let rest: Vec<CacheKey> = (2..MAX_SIZE).map(|index| key("srv", &url(index))).collect();
    rest.iter().for_each(|item| {
        assert!(cache.store(item, 0, Some(payload("rest")), TTL_SECONDS, now));
    });
    assert_eq!(entry_count(&cache), MAX_SIZE);
    let first_extra = key("srv", &url(MAX_SIZE));
    assert!(cache.store(&first_extra, 0, Some(payload("extra")), TTL_SECONDS, now));
    assert_eq!(cache.lookup(&early, now), None);
    assert_eq!(cache.lookup(&tied, now), Some(Some(payload("tied"))));
    let second_extra = key("srv", &url(200));
    assert!(cache.store(&second_extra, 0, Some(payload("extra")), TTL_SECONDS, now));
    assert_eq!(cache.lookup(&tied, now), None);
    assert_eq!(cache.lookup(&rest[0], now), Some(Some(payload("rest"))));
    assert_eq!(
        cache.lookup(&first_extra, now),
        Some(Some(payload("extra")))
    );
    assert_eq!(entry_count(&cache), MAX_SIZE);
}

#[tokio::test]
async fn store_is_refused_when_the_generation_moved_during_a_fetch() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let slot = cache.enter_fetch(&key).await;
    let read = cache.generation("srv");
    cache.invalidate("srv");
    assert_eq!(cache.generation("srv"), 1);
    assert!(!cache.store(
        &key,
        read,
        Some(payload("stale")),
        TTL_SECONDS,
        Instant::now()
    ));
    assert_eq!(cache.lookup(&key, Instant::now()), None);
    let fresh = cache.generation("srv");
    assert!(cache.store(
        &key,
        fresh,
        Some(payload("fresh")),
        TTL_SECONDS,
        Instant::now()
    ));
    assert_eq!(
        cache.lookup(&key, Instant::now()),
        Some(Some(payload("fresh")))
    );
    drop(slot);
}

#[tokio::test]
async fn store_is_accepted_while_the_generation_is_unchanged() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let slot = cache.enter_fetch(&key).await;
    let read = cache.generation("srv");
    assert!(cache.store(
        &key,
        read,
        Some(payload("fresh")),
        TTL_SECONDS,
        Instant::now()
    ));
    assert_eq!(
        cache.lookup(&key, Instant::now()),
        Some(Some(payload("fresh")))
    );
    drop(slot);
}

#[tokio::test]
async fn invalidate_removes_the_named_servers_entries_and_idle_locks_only() {
    let cache = OAuthMetadataCache::default();
    let now = Instant::now();
    let removed = key("srv-a", "https://idp.test/mcp");
    let kept = key("srv-b", "https://idp.test/mcp");
    assert!(cache.store(&removed, 0, Some(payload("a")), TTL_SECONDS, now));
    assert!(cache.store(&kept, 0, Some(payload("b")), TTL_SECONDS, now));
    let removed_slot = cache.enter_fetch(&removed).await;
    drop(removed_slot);
    let kept_slot = cache.enter_fetch(&kept).await;
    drop(kept_slot);
    cache.invalidate("srv-a");
    assert_eq!(cache.lookup(&removed, now), None);
    assert_eq!(cache.lookup(&kept, now), Some(Some(payload("b"))));
    assert!(!holds_lock(&cache, &removed));
    assert!(holds_lock(&cache, &kept));
    assert_eq!(cache.generation("srv-a"), 0);
    assert_eq!(cache.generation("srv-b"), 0);
}

#[tokio::test]
async fn invalidate_keeps_a_lock_a_fetch_still_holds() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let slot = cache.enter_fetch(&key).await;
    cache.invalidate("srv");
    assert!(holds_lock(&cache, &key));
    assert_eq!(cache.generation("srv"), 1);
    drop(slot);
    cache.invalidate("srv");
    assert!(!holds_lock(&cache, &key));
    assert_eq!(cache.generation("srv"), 0);
}

#[tokio::test]
async fn prune_drops_idle_locks_of_keys_without_entries_and_stale_generations() {
    let cache = OAuthMetadataCache::default();
    let now = Instant::now();
    let cached = key("srv", "https://idp.test/cached");
    let idle = key("srv", "https://idp.test/idle");
    let held = key("other", "https://idp.test/held");
    let cached_slot = cache.enter_fetch(&cached).await;
    assert!(cache.store(&cached, 0, Some(payload("cached")), TTL_SECONDS, now));
    drop(cached_slot);
    let idle_slot = cache.enter_fetch(&idle).await;
    drop(idle_slot);
    let held_slot = cache.enter_fetch(&held).await;
    cache.invalidate("other");
    assert_eq!(cache.generation("other"), 1);
    cache.prune(now);
    assert!(holds_lock(&cache, &cached));
    assert!(!holds_lock(&cache, &idle));
    assert!(holds_lock(&cache, &held));
    assert_eq!(cache.generation("other"), 1);
    drop(held_slot);
    cache.prune(now);
    assert!(!holds_lock(&cache, &held));
    assert_eq!(cache.generation("other"), 0);
}

#[tokio::test]
async fn two_waiters_on_one_key_enter_one_at_a_time() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let first = cache.enter_fetch(&key).await;
    let mut tasks = JoinSet::new();
    let second_cache = cache.clone();
    let second_key = key.clone();
    let (started, mut started_receiver) = mpsc::channel(1);
    let (entered, mut entered_receiver) = mpsc::channel(1);
    tasks.spawn(async move {
        started.send(()).await.unwrap();
        let _slot = second_cache.enter_fetch(&second_key).await;
        entered.send(()).await.unwrap();
    });
    started_receiver.recv().await.unwrap();
    tokio::task::yield_now().await;
    assert!(entered_receiver.try_recv().is_err());
    assert_eq!(fetcher_count(&cache, &key), 2);
    drop(first);
    entered_receiver.recv().await.unwrap();
    tasks.join_next().await.unwrap().unwrap();
    assert_eq!(fetcher_count(&cache, &key), 0);
}

#[tokio::test]
async fn a_cancelled_waiter_gives_its_fetcher_count_back() {
    let cache = OAuthMetadataCache::default();
    let key = key("srv", "https://idp.test/mcp");
    let first = cache.enter_fetch(&key).await;
    let mut tasks = JoinSet::new();
    let second_cache = cache.clone();
    let second_key = key.clone();
    let (started, mut started_receiver) = mpsc::channel(1);
    let handle = tasks.spawn(async move {
        started.send(()).await.unwrap();
        let _slot = second_cache.enter_fetch(&second_key).await;
    });
    started_receiver.recv().await.unwrap();
    assert_eq!(fetcher_count(&cache, &key), 2);
    handle.abort();
    let joined = tasks.join_next().await.unwrap();
    assert!(joined.unwrap_err().is_cancelled());
    assert_eq!(fetcher_count(&cache, &key), 1);
    drop(first);
    assert_eq!(fetcher_count(&cache, &key), 0);
    assert!(holds_lock(&cache, &key));
    cache.prune(Instant::now());
    assert!(!holds_lock(&cache, &key));
}
