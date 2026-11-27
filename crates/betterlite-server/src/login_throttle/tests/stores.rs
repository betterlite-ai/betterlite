use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_cache::RedisCache;
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

use crate::login_throttle::keys::LoginKeys;
use crate::login_throttle::local::{
    LocalStore, MAX_TRACKED_BLOCKS, MAX_TRACKED_COUNTERS, local_stores_with_limits,
};
use crate::login_throttle::redis::{FailureLimits, LoginRedis as _, RedisLogin};
use crate::login_throttle::support::FakeClock;

struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

async fn redis_cache(fixture: &RedisFixture) -> Arc<RedisCache> {
    let url = serde_json::to_string(fixture.as_ref()).expect("the fixture url");
    let overrides: RedisOverrides =
        serde_json::from_str(&format!(r#"{{"url":{url}}}"#)).expect("the redis overrides");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    let cache = connect(options, &BTreeMap::new())
        .await
        .expect("a connected cache");
    Arc::new(cache)
}

async fn login_redis(fixture: &RedisFixture) -> RedisLogin {
    let cache = redis_cache(fixture).await;
    RedisLogin::new(cache)
}

async fn raw_connection(fixture: &RedisFixture) -> MultiplexedConnection {
    Client::open(fixture.as_ref())
        .expect("the fixture client")
        .get_multiplexed_async_connection()
        .await
        .expect("a raw connection")
}

fn limits() -> FailureLimits {
    FailureLimits {
        user_limit: 2,
        source_limit: 4,
        window_seconds: 60,
        block_seconds: 300,
    }
}

// the registered _BLOCK_TTLS_LUA script over a real Redis: a missing key is EXPIRED (-2)
#[tokio::test]
async fn lua_block_ttls_reports_an_unblocked_pair() {
    let fixture = RedisFixture::start().await.expect("the redis fixture");
    let login = login_redis(&fixture).await;
    let keys = LoginKeys::of("10.1.2.3", "bob");
    let ttls = login
        .block_ttls(&keys)
        .await
        .expect("the block ttls script");
    assert_eq!(ttls.user, -2);
    assert_eq!(ttls.source, -2);
}

// the registered _RECORD_FAILURE_LUA script: the count over the limit sets the block and returns it
#[tokio::test]
async fn lua_record_failure_blocks_the_pair_on_the_triggering_count() {
    let fixture = RedisFixture::start().await.expect("the redis fixture");
    let login = login_redis(&fixture).await;
    let keys = LoginKeys::of("10.1.2.3", "bob");
    let first = login
        .record_failure(&keys, limits())
        .await
        .expect("the first failure");
    assert_eq!(first.user, 0);
    assert_eq!(first.source, 0);
    let second = login
        .record_failure(&keys, limits())
        .await
        .expect("the second failure");
    assert_eq!(second.user, 0);
    let third = login
        .record_failure(&keys, limits())
        .await
        .expect("the third failure");
    assert_eq!(third.user, 300);
    assert_eq!(third.source, 0);
    let blocked = login.block_ttls(&keys).await.expect("the blocked ttls");
    assert!((1..=300).contains(&blocked.user));
    assert_eq!(blocked.source, -2);
}

// the same script counts the source across usernames and blocks it on the count over its own limit
#[tokio::test]
async fn lua_record_failure_blocks_the_source_across_usernames() {
    let fixture = RedisFixture::start().await.expect("the redis fixture");
    let login = login_redis(&fixture).await;
    let bob = LoginKeys::of("10.1.2.3", "bob");
    login.record_failure(&bob, limits()).await.expect("bob one");
    login.record_failure(&bob, limits()).await.expect("bob two");
    let carol = LoginKeys::of("10.1.2.3", "carol");
    login
        .record_failure(&carol, limits())
        .await
        .expect("carol one");
    login
        .record_failure(&carol, limits())
        .await
        .expect("carol two");
    let dave = LoginKeys::of("10.1.2.3", "dave");
    let source_block = login
        .record_failure(&dave, limits())
        .await
        .expect("dave one");
    assert_eq!(source_block.user, 0);
    assert_eq!(source_block.source, 300);
    let ttls = login.block_ttls(&dave).await.expect("the source ttl");
    assert_eq!(ttls.user, -2);
    assert!((1..=300).contains(&ttls.source));
}

// clear_pair's shared delete removes the literal key the Lua scripts count under
#[tokio::test]
async fn delete_counter_removes_the_shared_pair_counter() {
    let fixture = RedisFixture::start().await.expect("the redis fixture");
    let login = login_redis(&fixture).await;
    let keys = LoginKeys::of("10.1.2.3", "bob");
    let pair_only = FailureLimits {
        source_limit: 0,
        ..limits()
    };
    login
        .record_failure(&keys, pair_only)
        .await
        .expect("a counted failure");
    let mut connection = raw_connection(&fixture).await;
    let before: Option<String> = cmd("GET")
        .arg(keys.pair_counter.as_str())
        .query_async(&mut connection)
        .await
        .expect("the counter read");
    assert!(before.is_some());
    login
        .delete_counter(&keys.pair_counter)
        .await
        .expect("the counter delete");
    let after: Option<String> = cmd("GET")
        .arg(keys.pair_counter.as_str())
        .query_async(&mut connection)
        .await
        .expect("the counter read");
    assert!(after.is_none());
}

// B125's bounded store keeps LiteLLM's capacities and evicts the earliest expiry once over them
#[test]
fn bounded_local_stores_evict_at_the_documented_capacities() {
    assert_eq!(MAX_TRACKED_COUNTERS, 20_000);
    assert_eq!(MAX_TRACKED_BLOCKS, 10_000);
    let clock = FakeClock::at(1000.0);
    let stores = local_stores_with_limits(clock.clock(), 2, 1).expect("the local stores");
    LocalStore::set(
        stores.counters.as_ref(),
        "first",
        CacheValue::Float(1.0),
        60,
    )
    .expect("the first count");
    clock.set(1001.0);
    LocalStore::set(
        stores.counters.as_ref(),
        "second",
        CacheValue::Float(2.0),
        60,
    )
    .expect("the second count");
    clock.set(1002.0);
    LocalStore::set(
        stores.counters.as_ref(),
        "third",
        CacheValue::Float(3.0),
        60,
    )
    .expect("the third count");
    assert!(
        LocalStore::get(stores.counters.as_ref(), "first")
            .expect("the first read")
            .is_none()
    );
    assert!(
        LocalStore::get(stores.counters.as_ref(), "second")
            .expect("the second read")
            .is_some()
    );
    assert!(
        LocalStore::get(stores.counters.as_ref(), "third")
            .expect("the third read")
            .is_some()
    );
    LocalStore::set(
        stores.blocks.as_ref(),
        "block-one",
        CacheValue::Float(1.0),
        60,
    )
    .expect("the first block");
    clock.set(1003.0);
    LocalStore::set(
        stores.blocks.as_ref(),
        "block-two",
        CacheValue::Float(2.0),
        60,
    )
    .expect("the second block");
    assert!(
        LocalStore::get(stores.blocks.as_ref(), "block-one")
            .expect("the first block read")
            .is_none()
    );
    assert!(
        LocalStore::get(stores.blocks.as_ref(), "block-two")
            .expect("the second block read")
            .is_some()
    );
    // the store's own counter and expiry, not just this crate's fake: an increment adds one
    assert_eq!(
        LocalStore::increment(stores.counters.as_ref(), "inc", 60)
            .expect("the first increment")
            .to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(
        LocalStore::increment(stores.counters.as_ref(), "inc", 60)
            .expect("the second increment")
            .to_bits(),
        2.0_f64.to_bits()
    );
    // a zero-second ttl has already expired once the clock moves past it
    LocalStore::set(stores.blocks.as_ref(), "short", CacheValue::Float(9.0), 0)
        .expect("the short block");
    clock.set(1003.5);
    assert!(
        LocalStore::get(stores.blocks.as_ref(), "short")
            .expect("the short read")
            .is_none()
    );
    LocalStore::set(stores.blocks.as_ref(), "doomed", CacheValue::Float(1.0), 60)
        .expect("the doomed block");
    LocalStore::delete(stores.blocks.as_ref(), "doomed").expect("the block delete");
    assert!(
        LocalStore::get(stores.blocks.as_ref(), "doomed")
            .expect("the deleted read")
            .is_none()
    );
}
