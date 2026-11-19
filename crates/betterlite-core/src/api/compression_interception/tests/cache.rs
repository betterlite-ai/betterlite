#![cfg(test)]

use super::support::{cache as content, manual_clock};
use crate::api::compression_interception::cache::CompressionCache;

const CALL_ID: &str = "call-1";

#[test]
fn a_stored_entry_carries_its_content_and_creation_time() {
    let (clock, _now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    let entry = cache.entry(CALL_ID).expect("the entry is stored");
    assert_eq!(entry.content, content(&[("key", "value")]));
    assert_eq!(entry.created_at, 100.0);
}

#[test]
fn an_unknown_call_id_has_no_entry() {
    let (clock, _now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    assert_eq!(cache.entry("missing"), None);
}

#[test]
fn pruning_keeps_a_younger_entry() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    *now.lock().expect("the clock lock") = 1099.0;
    cache.prune();
    assert!(cache.entry(CALL_ID).is_some());
}

#[test]
fn pruning_keeps_an_entry_exactly_nine_hundred_seconds_old() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    *now.lock().expect("the clock lock") = 1000.0;
    cache.prune();
    assert!(cache.entry(CALL_ID).is_some());
}

#[test]
fn pruning_drops_an_entry_older_than_nine_hundred_seconds() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    *now.lock().expect("the clock lock") = 1001.0;
    cache.prune();
    assert_eq!(cache.entry(CALL_ID), None);
}

#[test]
fn reading_an_entry_does_not_refresh_its_creation_time() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    *now.lock().expect("the clock lock") = 800.0;
    assert!(cache.entry(CALL_ID).is_some());
    *now.lock().expect("the clock lock") = 1001.0;
    cache.prune();
    assert_eq!(cache.entry(CALL_ID), None);
}

#[test]
fn storing_the_same_call_again_replaces_the_content_and_the_time() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store(CALL_ID.to_owned(), content(&[("key", "first")]));
    *now.lock().expect("the clock lock") = 200.0;
    cache.store(CALL_ID.to_owned(), content(&[("key", "second")]));
    let entry = cache.entry(CALL_ID).expect("the entry is stored");
    assert_eq!(entry.content, content(&[("key", "second")]));
    assert_eq!(entry.created_at, 200.0);
}

#[test]
fn pruning_leaves_only_the_expired_entries_out() {
    let (clock, now) = manual_clock(100.0);
    let cache = CompressionCache::with_clock(clock);
    cache.store("old".to_owned(), content(&[("key", "old")]));
    *now.lock().expect("the clock lock") = 500.0;
    cache.store("young".to_owned(), content(&[("key", "young")]));
    *now.lock().expect("the clock lock") = 1001.0;
    cache.prune();
    assert_eq!(cache.entry("old"), None);
    assert!(cache.entry("young").is_some());
}

#[test]
fn the_default_cache_stamps_from_the_wall_clock() {
    let cache = CompressionCache::default();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_secs_f64();
    cache.store(CALL_ID.to_owned(), content(&[("key", "value")]));
    let entry = cache.entry(CALL_ID).expect("the entry is stored");
    assert!(entry.created_at >= before);
}
