#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;

use super::super::sink::{ModelTableInvalidation, RedisModelTableInvalidation};
use super::support::capture_logs;

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct AllowingGate;

impl BreakerGate for AllowingGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

// A cache that opens without a server, so the publication fails on the connection and names what it tried to publish.
async fn unreachable_cache() -> RedisCache {
    let mut overrides: RedisOverrides =
        serde_json::from_str("{}").expect("the empty overrides deserialize");
    overrides.host = Some("127.0.0.1".to_owned());
    overrides.port = Some(IntOrText::Value(1_i64));
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60_i64,
        redis_flush_size: None,
        default_major_version: 7_u32,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(AllowingGate),
    };
    connect(options, &BTreeMap::new())
        .await
        .expect("the cache opens without a server")
}

// object_type="litellm_proxymodeltable" is what reaches publish_config_change: its failure warning names the object.
#[tokio::test]
async fn the_redis_sink_publishes_the_proxy_model_table_object_type() {
    let (logs, _guard) = capture_logs();
    let cache = unreachable_cache().await;
    let sink = RedisModelTableInvalidation {
        redis: Some(&cache),
    };
    sink.model_table_changed().await.unwrap();
    assert!(
        logs.contents()
            .contains("config sync publish for litellm_proxymodeltable failed")
    );
}

// Without a configured cache publish_config_change is a no-op, so the sink reports no failure.
#[tokio::test]
async fn a_missing_cache_publishes_nothing() {
    let (logs, _guard) = capture_logs();
    let sink = RedisModelTableInvalidation { redis: None };
    sink.model_table_changed().await.unwrap();
    assert!(!logs.contents().contains("config sync publish"));
}
