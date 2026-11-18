use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis_fixture::server::RedisFixture;
use redis::aio::MultiplexedConnection;
use redis::{Client, cmd};
use tokio::time::sleep;
use uuid::Uuid;

use crate::queues::pod_lock::cache::RedisLockCache;
use crate::queues::pod_lock::owner::PodId;
use crate::queues::pod_lock::{NoPodLockEventSink, PodLockManager};

const LEASE_TTL_SECONDS: i64 = 60;
const LEASE_KEY: &str = "cronjob_lock:job";
const EXPIRY_POLL_ATTEMPTS: usize = 500;
const EXPIRY_POLL_INTERVAL: Duration = Duration::from_millis(2);

struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

async fn cache_for(fixture: &RedisFixture) -> Arc<RedisLockCache> {
    let url = serde_json::to_string(fixture.url.as_str()).unwrap();
    let overrides: RedisOverrides = serde_json::from_str(&format!(r#"{{"url":{url}}}"#)).unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: LEASE_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    let env = BTreeMap::<String, String>::new();
    let cache = connect(options, &env).await.unwrap();
    Arc::new(RedisLockCache {
        cache: Arc::new(cache),
    })
}

fn pod_lock(cache: &Arc<RedisLockCache>, owner: Uuid) -> PodLockManager<RedisLockCache> {
    PodLockManager::new(
        Some(Arc::clone(cache)),
        LEASE_TTL_SECONDS,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(owner).unwrap(),
    )
}

async fn raw(fixture: &RedisFixture) -> MultiplexedConnection {
    Client::open(fixture.url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

async fn stored_owner(connection: &mut MultiplexedConnection) -> Option<String> {
    cmd("GET")
        .arg(LEASE_KEY)
        .query_async(connection)
        .await
        .unwrap()
}

async fn lease_ttl(connection: &mut MultiplexedConnection) -> i64 {
    cmd("PTTL")
        .arg(LEASE_KEY)
        .query_async(connection)
        .await
        .unwrap()
}

async fn wait_until_expired(connection: &mut MultiplexedConnection) {
    for _attempt in 0..EXPIRY_POLL_ATTEMPTS {
        if lease_ttl(connection).await == -2 {
            return;
        }
        sleep(EXPIRY_POLL_INTERVAL).await;
    }
    assert_eq!(lease_ttl(connection).await, -2, "the lease did not expire");
}

#[tokio::test]
async fn a_second_pod_acquires_only_after_the_holder_releases() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let first = pod_lock(&cache, Uuid::new_v4());
    let second = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(first.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(second.acquire_lock("job", None, true).await, Some(false));
    let mut connection = raw(&fixture).await;
    assert_eq!(
        stored_owner(&mut connection).await,
        Some(serde_json::to_string(first.pod_id.as_ref()).unwrap())
    );
    first.release_lock("job").await;
    assert_eq!(stored_owner(&mut connection).await, None);
    assert_eq!(second.acquire_lock("job", None, true).await, Some(true));
}

#[tokio::test]
async fn the_stored_owner_is_the_json_encoded_pod_id_with_the_lease_ttl() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let manager = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    let owner = manager.pod_id.as_ref();
    let mut connection = raw(&fixture).await;
    assert_eq!(
        stored_owner(&mut connection).await,
        Some(serde_json::to_string(owner).unwrap())
    );
    let stored = cache.cache.async_get_cache(LEASE_KEY).await.unwrap();
    assert!(matches!(stored, Some(CachedValue::Text(text)) if text == owner));
    let ttl = lease_ttl(&mut connection).await;
    assert!((1..=LEASE_TTL_SECONDS.saturating_mul(1000)).contains(&ttl));
}

#[tokio::test]
async fn a_reentrant_acquire_succeeds_without_renewing_the_expiry() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let manager = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    let mut connection = raw(&fixture).await;
    let shortened: i64 = cmd("PEXPIRE")
        .arg(LEASE_KEY)
        .arg(5000)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(shortened, 1);
    let before = lease_ttl(&mut connection).await;
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    let after = lease_ttl(&mut connection).await;
    assert!(after <= before, "the reentrant acquire renewed the lease");
    assert!(after > 0, "the reentrant acquire dropped the lease");
}

#[tokio::test]
async fn reentrancy_can_be_disabled_for_the_holder() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let manager = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(manager.acquire_lock("job", None, false).await, Some(false));
    let mut connection = raw(&fixture).await;
    assert_eq!(
        stored_owner(&mut connection).await,
        Some(serde_json::to_string(manager.pod_id.as_ref()).unwrap())
    );
    assert!(lease_ttl(&mut connection).await > 0);
}

#[tokio::test]
async fn a_release_by_a_pod_that_does_not_own_the_lock_keeps_the_key() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let holder = pod_lock(&cache, Uuid::new_v4());
    let stranger = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(holder.acquire_lock("job", None, true).await, Some(true));
    stranger.release_lock("job").await;
    let mut connection = raw(&fixture).await;
    assert_eq!(
        stored_owner(&mut connection).await,
        Some(serde_json::to_string(holder.pod_id.as_ref()).unwrap())
    );
    assert!(lease_ttl(&mut connection).await > 0);
    assert_eq!(stranger.acquire_lock("job", None, true).await, Some(false));
}

#[tokio::test]
async fn another_pod_acquires_after_the_key_expires() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let first = pod_lock(&cache, Uuid::new_v4());
    let second = pod_lock(&cache, Uuid::new_v4());
    assert_eq!(first.acquire_lock("job", None, true).await, Some(true));
    let mut connection = raw(&fixture).await;
    let shortened: i64 = cmd("PEXPIRE")
        .arg(LEASE_KEY)
        .arg(1)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(shortened, 1);
    wait_until_expired(&mut connection).await;
    assert_eq!(second.acquire_lock("job", None, true).await, Some(true));
}
