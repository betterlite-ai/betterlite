use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

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

use crate::auth_pubsub::channel::INVALIDATION_CHANNEL;
use crate::auth_pubsub::endpoint::{AuthPubSub, RedisAuthPubSub, TransportError};

use super::support::runtime;

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

async fn dead_cache(namespace: Option<&str>) -> RedisCache {
    let overrides: RedisOverrides = serde_json::from_str(r#"{"host":"127.0.0.1","port":1}"#)
        .expect("the dead redis settings decode");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    let env = BTreeMap::<String, String>::new();
    connect(options, &env)
        .await
        .expect("an unreachable redis still yields a cache")
}

fn redis_pubsub(cache: RedisCache) -> RedisAuthPubSub {
    RedisAuthPubSub {
        cache: Arc::new(cache),
    }
}

#[test]
fn redis_auth_pubsub_channel() {
    let runtime = runtime();
    let bare = redis_pubsub(runtime.block_on(dead_cache(None)));
    assert_eq!(bare.channel(), INVALIDATION_CHANNEL);
    let namespaced = redis_pubsub(runtime.block_on(dead_cache(Some("tenant"))));
    assert_eq!(
        namespaced.channel(),
        "tenant:litellm_proxy.auth_cache_invalidation"
    );
}

#[test]
fn redis_auth_pubsub_publish_reports_connection_failure() {
    let runtime = runtime();
    let pubsub = redis_pubsub(runtime.block_on(dead_cache(None)));
    let outcome = runtime.block_on(pubsub.publish("{}"));
    assert!(
        matches!(outcome, Err(TransportError::Connection(_))),
        "an unreachable redis fails the publish as a connection error"
    );
}

#[test]
fn redis_auth_pubsub_subscribe_reports_connection_failure() {
    let runtime = runtime();
    let pubsub = redis_pubsub(runtime.block_on(dead_cache(None)));
    let outcome = runtime.block_on(pubsub.subscribe());
    assert!(
        matches!(outcome, Err(TransportError::Connection(_))),
        "an unreachable redis fails the subscribe as a connection error"
    );
}
