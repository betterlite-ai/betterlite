#![cfg(test)]

use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use super::{CacheLookup, SecretCache};

fn second(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn full_cache(time_to_live: Duration, stored_at: fn(u64) -> Duration) -> SecretCache {
    let mut cache = SecretCache::new(time_to_live);
    for index in 0..200 {
        cache.store(&format!("secret-{index}"), None, stored_at(index));
    }
    cache
}

#[test]
fn cached_absence_is_a_hit_not_a_miss() {
    let mut cache = SecretCache::new(second(10));
    cache.store("missing", None, second(0));

    assert!(matches!(
        cache.lookup("missing", second(5)),
        CacheLookup::Hit(None)
    ));
}

#[test]
fn unknown_name_is_a_miss() {
    let mut cache = SecretCache::new(second(10));

    assert!(matches!(
        cache.lookup("unknown", second(0)),
        CacheLookup::Miss
    ));
}

#[test]
fn expired_absence_is_a_miss_after_the_time_to_live() {
    let mut cache = SecretCache::new(second(10));
    cache.store("missing", None, second(0));

    assert!(matches!(
        cache.lookup("missing", second(10)),
        CacheLookup::Hit(None)
    ));
    assert!(matches!(
        cache.lookup("missing", second(11)),
        CacheLookup::Miss
    ));
}

#[test]
fn stored_value_is_returned_until_it_expires() {
    let mut cache = SecretCache::new(second(10));
    cache.store("name", Some(SecretString::from("secret")), second(0));

    let CacheLookup::Hit(Some(found)) = cache.lookup("name", second(9)) else {
        panic!("expected a cached value");
    };
    assert_eq!(found.expose_secret(), "secret");
    assert!(matches!(
        cache.lookup("name", second(11)),
        CacheLookup::Miss
    ));
}

#[test]
fn rewriting_a_live_entry_keeps_its_original_expiry() {
    let mut cache = SecretCache::new(second(10));
    cache.store("name", None, second(0));
    cache.store("name", Some(SecretString::from("fresh")), second(8));

    assert!(matches!(
        cache.lookup("name", second(10)),
        CacheLookup::Hit(Some(_))
    ));
    assert!(matches!(
        cache.lookup("name", second(11)),
        CacheLookup::Miss
    ));
}

#[test]
fn rewriting_an_expired_entry_starts_a_new_time_to_live() {
    let mut cache = SecretCache::new(second(10));
    cache.store("name", None, second(0));
    cache.store("name", Some(SecretString::from("fresh")), second(20));

    assert!(matches!(
        cache.lookup("name", second(29)),
        CacheLookup::Hit(Some(_))
    ));
}

#[test]
fn full_cache_drops_the_earliest_expiring_entry() {
    let mut cache = full_cache(second(1000), second);

    cache.store("newcomer", None, second(300));

    assert!(matches!(
        cache.lookup("secret-0", second(300)),
        CacheLookup::Miss
    ));
    assert!(matches!(
        cache.lookup("secret-1", second(300)),
        CacheLookup::Hit(None)
    ));
    assert!(matches!(
        cache.lookup("newcomer", second(300)),
        CacheLookup::Hit(None)
    ));
}

#[test]
fn full_cache_drops_expired_entries_before_live_ones() {
    let mut cache = full_cache(second(10), |_| second(0));
    cache.store("late", None, second(5));

    cache.store("newcomer", None, second(11));

    assert!(matches!(
        cache.lookup("late", second(12)),
        CacheLookup::Hit(None)
    ));
    assert!(matches!(
        cache.lookup("secret-199", second(12)),
        CacheLookup::Miss
    ));
}
