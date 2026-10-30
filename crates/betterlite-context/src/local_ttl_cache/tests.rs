#![cfg(test)]

use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use super::{CacheError, CacheValue, LocalTtlCache, OrderedFloat, decode_cached_json};
use rstest::rstest;

fn cache(clock: f64) -> LocalTtlCache {
    LocalTtlCache::with_clock(
        Some(2),
        Some(600.0_f64),
        Some(1024),
        Arc::new(move || clock),
    )
    .expect("valid cache")
}

#[rstest]
#[case::cache_defaults("cache_defaults")]
#[case::expiry_and_overwrite("expiry_and_overwrite")]
#[case::nonrefreshing_ttl("nonrefreshing_ttl")]
#[case::value_size_policy("value_size_policy")]
#[case::cached_json_projection("cached_json_projection")]
#[case::dual_local_defaults_errors("dual_local_defaults_errors")]
fn local_ttl_cache_cases(#[case] name: &str) {
    match name {
        "cache_defaults" => {
            let defaults =
                LocalTtlCache::with_clock(None, None, None, Arc::new(|| 0.0_f64)).unwrap();
            assert_eq!(defaults.capacity(), 200);
            assert_eq!(defaults.default_ttl().to_bits(), 600.0_f64.to_bits());
            assert_eq!(defaults.max_size_per_item(), 1024);
            let mut disabled =
                LocalTtlCache::with_clock(Some(0), None, None, Arc::new(|| 0.0_f64)).unwrap();
            disabled
                .set("disabled", CacheValue::Integer(1), None)
                .unwrap();
            assert!(disabled.get("disabled").is_none());
            let mut bounded =
                LocalTtlCache::with_clock(None, None, None, Arc::new(|| 0.0_f64)).unwrap();
            bounded
                .set(&"k".repeat(1024), CacheValue::Integer(1), None)
                .unwrap();
            assert!(bounded.get(&"k".repeat(1024)).is_some());
            bounded
                .set(&"k".repeat(1025), CacheValue::Integer(1), None)
                .unwrap();
            assert!(bounded.get(&"k".repeat(1025)).is_none());
            assert_eq!(
                OrderedFloat(1.0_f64).partial_cmp(&OrderedFloat(2.0_f64)),
                Some(Ordering::Less)
            );
        }
        "expiry_and_overwrite" => {
            let mut cache = cache(10.0_f64);
            cache
                .set("b", CacheValue::Integer(2), Some(5.0_f64))
                .unwrap();
            cache
                .set("a", CacheValue::Integer(1), Some(5.0_f64))
                .unwrap();
            cache
                .set("c", CacheValue::Integer(3), Some(5.0_f64))
                .unwrap();
            assert_eq!(cache.expiry("b"), Some(15.0_f64));
            assert!(cache.get("a").is_none());
            assert!(cache.get("b").is_some());
            assert!(cache.get("c").is_some());
            let clock = Arc::new(Mutex::new(0.0_f64));
            let source = Arc::clone(&clock);
            let mut expiring = LocalTtlCache::with_clock(
                Some(3),
                Some(600.0_f64),
                Some(2048),
                Arc::new(move || *source.lock().unwrap()),
            )
            .unwrap();
            expiring
                .set("expired", CacheValue::Integer(1), Some(1.0_f64))
                .unwrap();
            *clock.lock().unwrap() = 2.0_f64;
            expiring
                .set("replacement", CacheValue::Integer(2), Some(5.0_f64))
                .unwrap();
            assert!(expiring.expiry("expired").is_none());
            assert_eq!(expiring.expiry("replacement"), Some(7.0_f64));
        }
        "nonrefreshing_ttl" => {
            let mut cache = cache(0.0_f64);
            cache
                .set("key", CacheValue::Integer(1), Some(10.0_f64))
                .unwrap();
            cache
                .set("key", CacheValue::Integer(2), Some(100.0_f64))
                .unwrap();
            assert_eq!(cache.expiry("key"), Some(10.0_f64));
            let equality_clock = Arc::new(Mutex::new(0.0_f64));
            let source = Arc::clone(&equality_clock);
            let mut equality = LocalTtlCache::with_clock(
                Some(2),
                Some(600.0_f64),
                Some(1024),
                Arc::new(move || *source.lock().unwrap()),
            )
            .unwrap();
            equality
                .set("key", CacheValue::Integer(1), Some(10.0_f64))
                .unwrap();
            *equality_clock.lock().unwrap() = 10.0_f64;
            equality.expiries.insert("key".to_owned(), 5.0_f64);
            equality
                .set("key", CacheValue::Integer(2), Some(100.0_f64))
                .unwrap();
            assert_eq!(equality.expiry("key"), Some(110.0_f64));
            cache.delete("key");
            assert!(cache.get("key").is_none());
            let clock = Arc::new(Mutex::new(0.0_f64));
            let source = Arc::clone(&clock);
            let mut boundary = LocalTtlCache::with_clock(
                Some(2),
                Some(600.0_f64),
                Some(1024),
                Arc::new(move || *source.lock().unwrap()),
            )
            .unwrap();
            boundary
                .set("boundary", CacheValue::Integer(1), Some(1.0_f64))
                .unwrap();
            *clock.lock().unwrap() = 1.0_f64;
            assert!(boundary.get("boundary").is_some());
            *clock.lock().unwrap() = 2.0_f64;
            assert!(boundary.get("boundary").is_none());
        }
        "value_size_policy" => {
            let mut cache =
                LocalTtlCache::with_clock(Some(2), Some(600.0_f64), Some(1), Arc::new(|| 0.0_f64))
                    .unwrap();
            cache
                .set("small", CacheValue::Text("ok".to_owned()), None)
                .unwrap();
            assert!(cache.get("small").is_some());
            cache
                .set("large", CacheValue::Text("x".repeat(2000)), None)
                .unwrap();
            assert!(cache.get("large").is_none());
            let mut ceiling = LocalTtlCache::with_clock(
                Some(2),
                Some(600.0_f64),
                Some(2048),
                Arc::new(|| 0.0_f64),
            )
            .unwrap();
            ceiling
                .set("ceiling", CacheValue::Text("x".repeat(2050)), None)
                .unwrap();
            assert!(ceiling.get("ceiling").is_some());
        }
        "cached_json_projection" => {
            assert_eq!(decode_cached_json(b"false"), CacheValue::Boolean(false));
            assert_eq!(
                decode_cached_json(b"not-json"),
                CacheValue::Text("not-json".to_owned())
            );
        }
        "dual_local_defaults_errors" => {
            let mut cache = cache(0.0_f64);
            cache
                .set("false", CacheValue::Boolean(false), None)
                .unwrap();
            assert_eq!(cache.get("false"), Some(CacheValue::Boolean(false)));
            assert_eq!(cache.get("missing"), None);
        }
        _ => assert!(name.is_empty()),
    }
}

#[test]
fn local_ttl_cache_credential_identity_isolation() {
    let mut cache = cache(0.0_f64);
    cache
        .set(
            "identity-a",
            CacheValue::Text("credential-a".to_owned()),
            None,
        )
        .unwrap();
    cache
        .set(
            "identity-b",
            CacheValue::Text("credential-b".to_owned()),
            None,
        )
        .unwrap();
    assert_ne!(cache.get("identity-a"), cache.get("identity-b"));
}

#[test]
fn local_ttl_cache_secret_safe_failure() {
    let error = LocalTtlCache::with_clock(Some(1), Some(600.0_f64), Some(0), Arc::new(|| 0.0_f64))
        .and_then(|mut cache| {
            cache.set("key", CacheValue::Text("fixture-secret".to_owned()), None)
        });
    let debug = format!("{error:?}");
    assert!(!debug.contains("fixture-secret"));
    assert_eq!(error, Ok(()));
    drop(CacheError::NegativeCapacity);
}

#[test]
fn replays_the_cached_json_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cached_json");
    let entries = fs::read_dir(corpus)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.len() >= 4);
    let decoded = entries
        .iter()
        .map(|entry| decode_cached_json(&fs::read(entry.path()).unwrap()));
    assert_eq!(decoded.count(), entries.len());
}
