#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::model_rate_limits as section;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_router::io_token_reservations::clock::IoClock;
use betterlite_router::model_rate_limits::cache::RateLimitCache;
use betterlite_router::prompt_affinity::service::{InjectionDefaults, TokenGate};
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::router_core::services::RouteServices;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{
    RedisSettings, RoutingStateStore, Settings as StateSettings,
};
use betterlite_router::routing_value_store::{
    RouteIncrement, RouteKey, RoutingValueStore, StoreSettings,
};

use super::{ModelRateLimitStores, RoutingRateLimitCache, build};
use crate::routing::prompt_affinity::PromptAffinityInputs;
use crate::routing::router_core::RouterServices;

fn local() -> Arc<RoutingValueStore> {
    let settings = StoreSettings {
        max_size_in_memory: Some(200),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    };
    Arc::new(
        RoutingValueStore::new(settings, Arc::new(|| 0.0)).expect("the store settings are valid"),
    )
}

fn stores(local: Arc<RoutingValueStore>) -> ModelRateLimitStores {
    ModelRateLimitStores {
        local,
        remote: None,
    }
}

fn cache(local: Arc<RoutingValueStore>) -> RoutingRateLimitCache {
    RoutingRateLimitCache {
        local,
        remote: None,
    }
}

fn clock() -> IoClock {
    Arc::new(|| UNIX_EPOCH + Duration::from_secs(1_700_000_000))
}

fn remote_settings(port: u16) -> StateSettings {
    StateSettings {
        redis: RedisSettings {
            url: None,
            host: Some(String::from("127.0.0.1")),
            port: Some(i64::from(port)),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    }
}

#[test]
fn build_requires_the_config_name() {
    let disabled = section::Settings::default();
    assert!(build(&disabled, stores(local()), None, clock()).is_none());
    let enabled = section::Settings {
        enforce_model_rate_limits: true,
    };
    assert!(build(&enabled, stores(local()), None, clock()).is_some());
}

#[test]
fn the_local_tier_counts_in_the_store() {
    let local = local();
    let cache = cache(Arc::clone(&local));
    assert_eq!(cache.increment_local("router:tpm", 2, 60), Some(2));
    assert_eq!(cache.increment_local("router:tpm", 3, 60), Some(5));
    assert_eq!(cache.local("router:tpm"), Some(5));
    assert_eq!(cache.local("router:absent"), None);
}

#[test]
fn an_absent_remote_keeps_the_local_count() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let cache = cache(local());
    assert!(!cache.has_remote());
    assert_eq!(
        runtime.block_on(cache.increment("router:rpm", 1, 60)),
        Some(1)
    );
    assert_eq!(
        runtime.block_on(cache.increment("router:rpm", 1, 60)),
        Some(2)
    );
    assert_eq!(
        runtime.block_on(cache.remote("router:rpm")),
        super::RemoteRead::Value(None)
    );
    assert_eq!(cache.local("router:rpm"), Some(2));
}

#[test]
fn a_configured_remote_answers_reads() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let server = runtime
        .block_on(FakeRedis::try_start())
        .expect("the fake redis starts");
    let store =
        RoutingStateStore::new(&remote_settings(server.port()), Arc::new(|| Duration::ZERO))
            .expect("the state store opens");
    let cache = RoutingRateLimitCache {
        local: local(),
        remote: Some(Arc::new(store)),
    };
    assert!(cache.has_remote());
    assert_eq!(
        runtime.block_on(cache.remote("router:tpm")),
        super::RemoteRead::Value(None)
    );
}

#[test]
fn the_remote_counter_wins_when_it_answers() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let server = runtime
        .block_on(FakeRedis::try_start())
        .expect("the fake redis starts");
    let store =
        RoutingStateStore::new(&remote_settings(server.port()), Arc::new(|| Duration::ZERO))
            .expect("the state store opens");
    let key = RouteKey::parse("router:rpm").expect("the key is valid");
    runtime
        .block_on(store.increment(&key, &RouteIncrement::Counter(5), None))
        .expect("the remote seed lands");
    let cache = RoutingRateLimitCache {
        local: local(),
        remote: Some(Arc::new(store)),
    };
    assert_eq!(
        runtime.block_on(cache.increment("router:rpm", 1, 60)),
        Some(6)
    );
    assert_eq!(cache.local("router:rpm"), Some(1));
}

#[test]
fn an_oversized_key_is_not_counted() {
    let cache = cache(local());
    let key = "k".repeat(2048);
    assert_eq!(cache.local(&key), None);
    assert_eq!(cache.increment_local(&key, 1, 60), None);
}

struct AllowAll;

#[async_trait]
impl TokenGate for AllowAll {
    async fn meets_minimum(&self, _model: &str, _messages: &[Json], _minimum: Option<u64>) -> bool {
        true
    }
}

struct NoFacts;

impl ModelFacts for NoFacts {
    fn cache_breakpoint_flag(&self, _model: &str) -> Option<bool> {
        None
    }

    fn provider(&self, _model: &str) -> Option<String> {
        None
    }
}

impl CacheFacts for NoFacts {
    fn supports_prompt_caching(&self, _model: &str, _provider: &str) -> bool {
        false
    }
}

// the config names no optional check, so the prompt caching check is never built and never reads these inputs
fn inputs() -> PromptAffinityInputs<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 0.0_f64);
    let memory = LocalMemoryOps::new(1024, clock).expect("the memory cache opens");
    PromptAffinityInputs {
        cache: Arc::new(DualCache::new(
            Arc::new(memory),
            None,
            DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 1000,
                redis_delete_chunk_size: 1000,
            },
        )),
        gate: Arc::new(AllowAll),
        facts: Arc::new(NoFacts),
        catalog: Arc::new(
            ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog parses"),
        ),
        settings: PromptCacheSettings::default(),
        defaults: InjectionDefaults::default(),
    }
}

fn router_services() -> RouterServices {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let config = ConfigLoader { lookup }
        .load("model_list: []\nrouter_settings:\n  optional_pre_call_checks: []\n")
        .expect("the test config loads");
    RouterServices::from_config(&config, inputs()).expect("the router builds")
}

// Router.add_optional_pre_call_checks: RouterServices carries the check the server registered, and none when it registered none
#[test]
fn the_router_services_expose_the_registered_check() {
    assert!(router_services().model_rate_limits().is_none());
    let check = build(
        &section::Settings {
            enforce_model_rate_limits: true,
        },
        stores(local()),
        None,
        clock(),
    )
    .expect("the check is registered");
    let services = router_services().with_model_rate_limits(Some(check));
    assert!(services.model_rate_limits().is_some());
}
