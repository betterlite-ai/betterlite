#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
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
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::io_token_reservations::clock::{IoClock, minute_label};
use betterlite_router::model_rate_limits::cache::RateLimitCache;
use betterlite_router::model_rate_limits::error::{ModelRateLimitError, RateLimitChannel};
use betterlite_router::model_rate_limits::keys::cache_keys;
use betterlite_router::prompt_affinity::service::{InjectionDefaults, TokenGate};
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::router_core::services::RouteServices;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{
    RedisSettings, RoutingStateStore, Settings as StateSettings,
};
use betterlite_router::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead, StoreSettings,
};
use betterlite_tokenizer::reservation::RequestBody;
use serde_json::Number;
use tokio::runtime::Builder;

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
    let cache = cache(local);
    assert_eq!(cache.increment_local("router:tpm", 2, 60), Some(2));
    assert_eq!(cache.increment_local("router:tpm", 3, 60), Some(5));
    assert_eq!(cache.local("router:tpm"), Some(5));
    assert_eq!(cache.local("router:absent"), None);
}

#[test]
fn an_absent_remote_keeps_the_local_count() {
    let runtime = Builder::new_current_thread()
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
    let runtime = Builder::new_current_thread()
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
    let runtime = Builder::new_current_thread()
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

fn limited_deployment() -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(
        String::from("model"),
        Json::Text(String::from("openai/gpt-4o")),
    );
    params.insert(String::from("tpm"), Json::Number(Number::from(5_i64)));
    params.insert(String::from("rpm"), Json::Number(Number::from(1_i64)));
    let mut info = JsonObject::new();
    info.insert(String::from("id"), Json::Text(String::from("a")));
    DeploymentSpec {
        model_name: String::from("gpt-4o"),
        litellm_params: params,
        model_info: info,
        extra: JsonObject::new(),
    }
}

// RedisCache.async_get_cache answers None when the connection fails, so the rpm increment still runs and rejects
#[test]
fn a_redis_outage_keeps_the_rpm_check() {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let server = runtime
        .block_on(FakeRedis::try_start())
        .expect("the fake redis starts");
    let store =
        RoutingStateStore::new(&remote_settings(server.port()), Arc::new(|| Duration::ZERO))
            .expect("the state store opens");
    let local = local();
    let deployment = limited_deployment();
    let keys = cache_keys(&deployment, &minute_label(clock()()));
    let rpm = RouteKey::parse(&keys.rpm).expect("the rpm key parses");
    local
        .increment(&rpm, &RouteIncrement::Counter(1), None)
        .expect("the local rpm counter seeds");
    let check = build(
        &section::Settings {
            enforce_model_rate_limits: true,
        },
        ModelRateLimitStores {
            local,
            remote: Some(Arc::new(store)),
        },
        None,
        clock(),
    )
    .expect("the check is registered");
    server.set_down(true);
    let error = runtime
        .block_on(check.async_pre_call_check(&deployment, &mut RequestBody::new()))
        .expect_err("the rpm limit rejects during the outage");
    let ModelRateLimitError::Exceeded(exceeded) = error else {
        panic!("expected a rate limit, got {error:?}");
    };
    assert_eq!(exceeded.channel, RateLimitChannel::Rpm);
    assert_eq!(exceeded.current, 2);
}

// an open breaker is its own answer, so the service falls back to the in-memory count instead of skipping the check
#[test]
fn an_open_breaker_answers_reads_as_its_own_state() {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let server = runtime
        .block_on(FakeRedis::try_start())
        .expect("the fake redis starts");
    let mut settings = remote_settings(server.port());
    settings.breaker.failure_threshold = 1;
    let store = RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO))
        .expect("the state store opens");
    let cache = RoutingRateLimitCache {
        local: local(),
        remote: Some(Arc::new(store)),
    };
    server.set_down(true);
    assert_eq!(
        runtime.block_on(cache.remote("router:tpm")),
        super::RemoteRead::Value(None)
    );
    assert_eq!(
        runtime.block_on(cache.remote("router:tpm")),
        super::RemoteRead::BreakerOpen
    );
}

// a value the counter check cannot use is a failed read, so the service lets the request through instead of rejecting
#[test]
fn a_non_counter_remote_value_reads_as_failed() {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds");
    let server = runtime
        .block_on(FakeRedis::try_start())
        .expect("the fake redis starts");
    let store =
        RoutingStateStore::new(&remote_settings(server.port()), Arc::new(|| Duration::ZERO))
            .expect("the state store opens");
    let key = RouteKey::parse("router:rpm").expect("the key parses");
    runtime
        .block_on(store.compare_and_set(
            &key,
            &StoreRead::Missing,
            &RouteValue::Members(BTreeSet::new()),
            None,
        ))
        .expect("the member value lands");
    let cache = RoutingRateLimitCache {
        local: local(),
        remote: Some(Arc::new(store)),
    };
    assert_eq!(
        runtime.block_on(cache.remote("router:rpm")),
        super::RemoteRead::Failed
    );
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
