#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_live_client::LiveClientFactory;

use crate::redis_cache_wiring::build::{RedisCacheRequest, RedisCacheWiring};

use super::support::{
    FakeClock, NoopHooks, closed_port, connection_error, empty_environment, overrides, settings,
};

const DEFAULT_TTL: i64 = 120;
const MAJOR_VERSION: u32 = 7;

fn wiring(failure_threshold: u32) -> RedisCacheWiring {
    RedisCacheWiring {
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoopHooks),
        policy: settings(failure_threshold),
        clock: Arc::new(FakeClock::new()).handle(),
        default_ttl: DEFAULT_TTL,
        default_major_version: MAJOR_VERSION,
    }
}

fn request(namespace: Option<&str>) -> RedisCacheRequest {
    RedisCacheRequest {
        overrides: overrides(closed_port()),
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        redis_flush_size: Some(50),
    }
}

#[tokio::test]
async fn build_returns_a_cache_while_redis_is_unreachable() {
    let wiring = wiring(3);
    let environment = empty_environment();
    let cache = wiring
        .build(request(Some("cache")), &environment)
        .await
        .expect("the cache builds without a reachable redis");
    assert_eq!(cache.default_ttl, DEFAULT_TTL);
    assert_eq!(cache.namespace.as_deref(), Some("cache"));
    assert_eq!(cache.redis_major_version, MAJOR_VERSION);
    assert_eq!(cache.transport.family, RedisCacheFamily::Standalone);
}

#[tokio::test]
async fn two_caches_get_two_gates() {
    let wiring = wiring(1);
    let environment = empty_environment();
    let first = wiring
        .build(request(None), &environment)
        .await
        .expect("the first cache builds");
    let second = wiring
        .build(request(None), &environment)
        .await
        .expect("the second cache builds");
    assert!(!Arc::ptr_eq(&first.gate, &second.gate));
    let error = connection_error();
    first.gate.record_failure(&error);
    assert!(
        first.gate.enter("async_get_cache").is_err(),
        "the first cache's breaker opened"
    );
    second
        .gate
        .enter("async_get_cache")
        .expect("the second cache's breaker stays closed");
}
