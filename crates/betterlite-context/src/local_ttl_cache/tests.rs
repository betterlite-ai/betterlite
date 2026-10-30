#![cfg(test)]

use std::fs;
use std::sync::Arc;

use super::{CacheError, CacheValue, LocalTtlCache, decode_cached_json};
use rstest::rstest;

fn cache(clock: f64) -> LocalTtlCache {
    LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1024), Arc::new(move || clock))
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
            let defaults = LocalTtlCache::with_clock(None, None, None, Arc::new(|| 0.0)).unwrap();
            assert_eq!(defaults.capacity(), 200);
            assert_eq!(defaults.default_ttl(), 600.0);
            assert_eq!(defaults.max_size_per_item(), 1024);
        }
        "expiry_and_overwrite" => {
            let mut cache = cache(10.0);
            cache.set("b", CacheValue::Integer(2), Some(5.0)).unwrap();
            cache.set("a", CacheValue::Integer(1), Some(5.0)).unwrap();
            cache.set("c", CacheValue::Integer(3), Some(5.0)).unwrap();
            assert_eq!(cache.expiry("b"), Some(15.0));
            assert!(cache.get("a").is_none());
            assert!(cache.get("b").is_some());
            assert!(cache.get("c").is_some());
        }
        "nonrefreshing_ttl" => {
            let mut cache = cache(0.0);
            cache
                .set("key", CacheValue::Integer(1), Some(10.0))
                .unwrap();
            cache
                .set("key", CacheValue::Integer(2), Some(100.0))
                .unwrap();
            assert_eq!(cache.expiry("key"), Some(10.0));
            cache.delete("key");
            assert!(cache.get("key").is_none());
        }
        "value_size_policy" => {
            let mut cache =
                LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1), Arc::new(|| 0.0)).unwrap();
            cache
                .set("small", CacheValue::Text("ok".to_owned()), None)
                .unwrap();
            assert!(cache.get("small").is_some());
            cache
                .set("large", CacheValue::Text("x".repeat(2000)), None)
                .unwrap();
            assert!(cache.get("large").is_none());
        }
        "cached_json_projection" => {
            assert_eq!(decode_cached_json(b"false"), CacheValue::Boolean(false));
            assert_eq!(
                decode_cached_json(b"not-json"),
                CacheValue::Text("not-json".to_owned())
            );
        }
        "dual_local_defaults_errors" => {
            let mut cache = cache(0.0);
            cache
                .set("false", CacheValue::Boolean(false), None)
                .unwrap();
            assert_eq!(cache.get("false"), Some(CacheValue::Boolean(false)));
            assert_eq!(cache.get("missing"), None);
        }
        _ => unreachable!(),
    }
}

#[test]
fn local_ttl_cache_credential_identity_isolation() {
    let mut cache = cache(0.0);
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
    let error = LocalTtlCache::with_clock(Some(1), Some(600.0), Some(0), Arc::new(|| 0.0))
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
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cached_json");
    let entries = fs::read_dir(corpus)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.len() >= 4);
    entries.iter().for_each(|entry| {
        drop(decode_cached_json(&fs::read(entry.path()).unwrap()));
    });
}
