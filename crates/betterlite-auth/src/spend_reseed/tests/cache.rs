use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::credential::{Credential, CredentialHandle};
use betterlite_redis::cache_primitives::decode::CachedValue;
use secrecy::SecretString;

use super::support::decimal;
use crate::spend_reseed::cache::ReseedCache as _;
use crate::spend_reseed::cache::dual::{memory_value, redis_value};
use crate::spend_reseed::values::ReseedCacheValue;

fn settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    }
}

fn local_cache() -> (Arc<LocalMemoryOps>, DualCache<LocalMemoryOps>) {
    let memory = Arc::new(
        LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
    );
    let cache = DualCache::new(Arc::clone(&memory), None, settings());
    (memory, cache)
}

#[tokio::test]
async fn dual_cache_memory_tier_round_trips_values() {
    let (_memory, cache) = local_cache();
    assert!(!cache.redis_configured());
    assert!(matches!(cache.redis_get("spend:key:t").await, Ok(None)));
    assert!(matches!(
        cache.redis_set_nx("spend:key:t", decimal("2.5")).await,
        Ok(None)
    ));
    let missing = cache.memory_get("spend:key:t").await.expect("the read");
    assert_eq!(missing, None);

    cache
        .memory_set("spend:key:t", decimal("2.5"))
        .await
        .expect("the memory write");
    let stored = cache.memory_get("spend:key:t").await.expect("the read");
    assert_eq!(stored, Some(ReseedCacheValue::Number(decimal("2.5"))));

    let first = cache
        .increment_local("spend:key:t", decimal("1.5"))
        .await
        .expect("the local increment");
    assert_eq!(first, Some(decimal("4")));
    let second = cache
        .increment_local("spend:key:t", decimal("-0.5"))
        .await
        .expect("the local increment");
    assert_eq!(second, Some(decimal("3.5")));
}

#[tokio::test]
async fn memory_values_map_to_cache_amounts() {
    let (memory, cache) = local_cache();
    let cases = [
        (
            "boolean",
            CacheValue::Boolean(true),
            Some(ReseedCacheValue::Boolean(true)),
        ),
        (
            "integer",
            CacheValue::Integer(7),
            Some(ReseedCacheValue::Number(decimal("7"))),
        ),
        (
            "float",
            CacheValue::Float(2.5),
            Some(ReseedCacheValue::Number(decimal("2.5"))),
        ),
        (
            "text",
            CacheValue::Text("abc".to_owned()),
            Some(ReseedCacheValue::Text("abc".to_owned())),
        ),
        (
            "json text",
            CacheValue::Text("1.5".to_owned()),
            Some(ReseedCacheValue::Number(decimal("1.5"))),
        ),
        ("null", CacheValue::Null, None),
        (
            "array",
            CacheValue::Array(vec![CacheValue::Integer(1)]),
            Some(ReseedCacheValue::Unreadable),
        ),
        (
            "object",
            CacheValue::Object(BTreeMap::new()),
            Some(ReseedCacheValue::Unreadable),
        ),
        (
            "bytes",
            CacheValue::Bytes(vec![1, 2]),
            Some(ReseedCacheValue::Unreadable),
        ),
    ];
    for (key, value, expected) in cases {
        memory.set_cache(key, value, None).expect("the raw value");
        let mapped = cache.memory_get(key).await.expect("the read");
        assert_eq!(mapped, expected);
    }
}

#[test]
fn redis_values_map_to_cache_amounts() {
    assert_eq!(redis_value(CachedValue::Null), None);
    assert_eq!(
        redis_value(CachedValue::Boolean(true)),
        Some(ReseedCacheValue::Boolean(true))
    );
    assert_eq!(
        redis_value(CachedValue::Number(
            serde_json::Number::from_f64(2.5).expect("a finite number")
        )),
        Some(ReseedCacheValue::Number(decimal("2.5")))
    );
    assert_eq!(
        redis_value(CachedValue::Text("2.5".to_owned())),
        Some(ReseedCacheValue::Text("2.5".to_owned()))
    );
    assert_eq!(
        redis_value(CachedValue::Array(Vec::new())),
        Some(ReseedCacheValue::Unreadable)
    );
    assert_eq!(
        redis_value(CachedValue::Object(BTreeMap::new())),
        Some(ReseedCacheValue::Unreadable)
    );
    let credential = CredentialHandle(Arc::new(Credential {
        token: SecretString::from("token".to_owned()),
        headers: BTreeMap::new(),
    }));
    assert_eq!(
        memory_value(CacheValue::Credential(credential)),
        Some(ReseedCacheValue::Unreadable)
    );
}
