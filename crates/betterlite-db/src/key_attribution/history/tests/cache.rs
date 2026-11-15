use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::advance;

use crate::key_attribution::history::cache::{MISS_TTL, MetadataCache, TTL};
use crate::key_attribution::history::window::spend_log_cache_key;
use crate::key_attribution::metadata::KeyMetadata;

use super::support::{DIGEST, OTHER_DIGEST, named, window};

const MILLISECOND: Duration = Duration::from_millis(1);

fn small_cache(capacity: usize) -> MetadataCache {
    MetadataCache {
        capacity,
        entries: Mutex::new(BTreeMap::new()),
    }
}

#[tokio::test(start_paused = true)]
async fn a_first_miss_is_cached_empty_for_exactly_thirty_seconds() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let key = spend_log_cache_key(DIGEST, &probed);
    cache.remember(DIGEST, &probed, None);
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
    advance(MISS_TTL).await;
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
    advance(MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
}

#[tokio::test(start_paused = true)]
async fn a_repeated_miss_while_the_marker_lives_is_cached_for_the_full_ttl() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let key = spend_log_cache_key(DIGEST, &probed);
    cache.remember(DIGEST, &probed, None);
    advance(MISS_TTL + MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
    cache.remember(DIGEST, &probed, None);
    advance(MISS_TTL + MISS_TTL).await;
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
    advance(Duration::from_secs(540)).await;
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
    advance(MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
}

#[tokio::test(start_paused = true)]
async fn a_miss_after_the_marker_expired_is_a_first_miss_again() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let key = spend_log_cache_key(DIGEST, &probed);
    cache.remember(DIGEST, &probed, None);
    advance(TTL + MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
    cache.remember(DIGEST, &probed, None);
    advance(MISS_TTL).await;
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
    advance(MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
}

#[tokio::test(start_paused = true)]
async fn a_success_is_cached_for_the_default_ttl() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let key = spend_log_cache_key(DIGEST, &probed);
    cache.remember(DIGEST, &probed, Some(&named("found")));
    advance(TTL).await;
    assert_eq!(cache.get(&key), Some(named("found")));
    advance(MILLISECOND).await;
    assert_eq!(cache.get(&key), None);
}

#[tokio::test(start_paused = true)]
async fn a_success_does_not_clear_the_missed_before_marker() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let key = spend_log_cache_key(DIGEST, &probed);
    cache.remember(DIGEST, &probed, None);
    advance(MISS_TTL + MILLISECOND).await;
    cache.remember(DIGEST, &probed, Some(&named("found")));
    cache
        .entries
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&key);
    cache.remember(DIGEST, &probed, None);
    advance(MISS_TTL + MISS_TTL).await;
    assert_eq!(cache.get(&key), Some(KeyMetadata::default()));
}

#[tokio::test(start_paused = true)]
async fn windows_and_digests_never_share_cache_entries() {
    let cache = MetadataCache::default();
    let probed = window(0, 60);
    let later = window(0, 61);
    cache.remember(DIGEST, &probed, Some(&named("found")));
    let both = BTreeSet::from([DIGEST.to_owned(), OTHER_DIGEST.to_owned()]);
    assert_eq!(
        cache.cached(&both, &probed),
        BTreeMap::from([(DIGEST.to_owned(), named("found"))])
    );
    assert!(cache.cached(&both, &later).is_empty());
    assert!(cache.cached(&both, &window(1, 60)).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_live_entry_keeps_its_deadline_when_it_is_set_again() {
    let cache = small_cache(10);
    cache.set("key", named("first"), Duration::from_secs(10));
    advance(Duration::from_secs(5)).await;
    cache.set("key", named("second"), Duration::from_secs(100));
    assert_eq!(cache.get("key"), Some(named("second")));
    advance(Duration::from_secs(5)).await;
    assert_eq!(cache.get("key"), Some(named("second")));
    advance(MILLISECOND).await;
    assert_eq!(cache.get("key"), None);
}

#[tokio::test(start_paused = true)]
async fn an_expired_entry_gets_a_new_deadline_when_it_is_set_again() {
    let cache = small_cache(10);
    cache.set("key", named("first"), Duration::from_secs(10));
    advance(Duration::from_secs(11)).await;
    cache.set("key", named("second"), Duration::from_secs(100));
    advance(Duration::from_secs(99)).await;
    assert_eq!(cache.get("key"), Some(named("second")));
}

#[tokio::test(start_paused = true)]
async fn a_full_cache_evicts_the_entry_that_expires_first() {
    let cache = small_cache(2);
    cache.set("a", named("a"), Duration::from_secs(10));
    cache.set("b", named("b"), Duration::from_secs(20));
    cache.set("c", named("c"), Duration::from_secs(30));
    assert_eq!(cache.get("a"), None);
    assert_eq!(cache.get("b"), Some(named("b")));
    assert_eq!(cache.get("c"), Some(named("c")));
}

#[tokio::test(start_paused = true)]
async fn equal_deadlines_evict_the_smaller_key_first() {
    let cache = small_cache(2);
    cache.set("b", named("b"), Duration::from_secs(10));
    cache.set("a", named("a"), Duration::from_secs(10));
    cache.set("c", named("c"), Duration::from_secs(10));
    assert_eq!(cache.get("a"), None);
    assert_eq!(cache.get("b"), Some(named("b")));
    assert_eq!(cache.get("c"), Some(named("c")));
}

#[tokio::test(start_paused = true)]
async fn expired_entries_leave_before_a_live_one_is_evicted() {
    let cache = small_cache(2);
    cache.set("short", named("short"), Duration::from_secs(5));
    cache.set("long", named("long"), Duration::from_secs(50));
    advance(Duration::from_secs(6)).await;
    cache.set("new", named("new"), Duration::from_secs(50));
    assert_eq!(cache.get("long"), Some(named("long")));
    assert_eq!(cache.get("new"), Some(named("new")));
}

#[tokio::test(start_paused = true)]
async fn entries_expiring_exactly_now_are_pruned_before_a_new_one_is_stored() {
    let cache = small_cache(3);
    cache.set("x", named("x"), Duration::from_secs(5));
    cache.set("y", named("y"), Duration::from_secs(5));
    cache.set("z", named("z"), Duration::from_secs(50));
    advance(Duration::from_secs(5)).await;
    cache.set("new", named("new"), Duration::from_secs(50));
    assert_eq!(cache.get("y"), None);
    assert_eq!(cache.get("z"), Some(named("z")));
    assert_eq!(cache.get("new"), Some(named("new")));
}

#[tokio::test(start_paused = true)]
async fn overwriting_an_entry_in_a_full_cache_still_evicts_the_earliest_one() {
    let cache = small_cache(2);
    cache.set("a", named("a"), Duration::from_secs(10));
    cache.set("b", named("b"), Duration::from_secs(20));
    cache.set("b", named("again"), Duration::from_secs(20));
    assert_eq!(cache.get("a"), None);
    assert_eq!(cache.get("b"), Some(named("again")));
}

#[tokio::test(start_paused = true)]
async fn a_zero_capacity_cache_stores_nothing() {
    let cache = small_cache(0);
    cache.set("key", named("value"), TTL);
    assert_eq!(cache.get("key"), None);
}
