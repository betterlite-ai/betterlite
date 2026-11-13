#![cfg(test)]

use super::is_redis_cluster;
use crate::rate_limit_window_store::RedisCacheKind;
use crate::rate_limit_window_store::tests::support::FakeCache;

#[test]
fn a_cluster_cache_is_a_redis_cluster() {
    let cache = FakeCache::new(Some(RedisCacheKind::Cluster));
    assert!(is_redis_cluster(cache.as_ref()));
}

#[test]
fn a_standalone_cache_is_not_a_redis_cluster() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    assert!(!is_redis_cluster(cache.as_ref()));
}

#[test]
fn a_cache_without_redis_is_not_a_redis_cluster() {
    let cache = FakeCache::new(None);
    assert!(!is_redis_cluster(cache.as_ref()));
}
