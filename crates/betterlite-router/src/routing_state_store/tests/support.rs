use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use secrecy::SecretString;

use super::super::{BreakerClock, RedisSettings, RoutingStateStore, Settings};
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_value_store::{Clock, RouteKey, RoutingValueStore, StoreSettings};

use super::fake_redis::FakeRedis;

pub(crate) struct TestClock(Arc<AtomicU64>);

impl TestClock {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    pub(crate) fn set_millis(&self, millis: u64) {
        self.0.store(millis, Ordering::SeqCst);
    }

    pub(crate) fn breaker_clock(&self) -> BreakerClock {
        let source = Arc::clone(&self.0);
        Arc::new(move || Duration::from_millis(source.load(Ordering::SeqCst)))
    }
}

pub(crate) fn route_key(raw: &str) -> RouteKey {
    RouteKey::parse(raw).unwrap()
}

pub(crate) fn local_store() -> RoutingValueStore {
    let clock: Clock = Arc::new(|| 0.0_f64);
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: None,
            loaded_max_size_per_item_kib: 1024,
        },
        clock,
    )
    .unwrap()
}

pub(crate) fn breaker(threshold: u32, recovery_millis: u64) -> BreakerSettings {
    BreakerSettings {
        enabled: true,
        failure_threshold: threshold,
        recovery_timeout: Duration::from_millis(recovery_millis),
        timeout_min_duration: Duration::from_millis(5_000),
        timeout_log_interval: Duration::from_millis(1_000),
    }
}

pub(crate) fn redis_settings(server: &FakeRedis, namespace: Option<&str>) -> RedisSettings {
    RedisSettings {
        url: None,
        host: Some("127.0.0.1".to_owned()),
        port: Some(i64::from(server.port())),
        password: None,
        namespace: namespace.map(str::to_owned),
        socket_timeout: Some(1.0_f64),
        pool_timeout_seconds: 5,
        default_ttl: 60,
    }
}

pub(crate) fn store(
    server: &FakeRedis,
    namespace: Option<&str>,
    breaker_settings: BreakerSettings,
    clock: &TestClock,
) -> RoutingStateStore {
    let settings = Settings {
        redis: Some(redis_settings(server, namespace)),
        breaker: breaker_settings,
    };
    RoutingStateStore::new(local_store(), &settings, clock.breaker_clock()).unwrap()
}

pub(crate) fn local_only_store(clock: &TestClock) -> RoutingStateStore {
    let settings = Settings {
        redis: None,
        breaker: breaker(5, 60_000),
    };
    RoutingStateStore::new(local_store(), &settings, clock.breaker_clock()).unwrap()
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn same_f64(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}
