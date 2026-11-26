#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_config::types::Config;
use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_prompts::cache_accounting::dialect::ModelFacts;
use betterlite_prompts::cache_defaults::CacheFacts;
use betterlite_router::prompt_affinity::service::{InjectionDefaults, TokenGate};
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::request_scheduler::flow::FlowItem;
use betterlite_router::router_core::services::RouteServices as _;
use betterlite_router::routing_state_store::Settings as SharedSettings;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{RedisSettings, RoutingStateStore};
use betterlite_router::routing_value_store::{RouteKey, RouteValue, StoreRead};

use crate::routing::prompt_affinity::PromptAffinityInputs;
use crate::routing::router_core::RouterServices;

use super::build;

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("the fixture config loads")
}

fn redis_config(port: u16) -> Config {
    config(&format!(
        "router_settings:\n  redis_host: 127.0.0.1\n  redis_port: {port}\n"
    ))
}

fn item(priority: u8, request_id: &str) -> FlowItem {
    FlowItem {
        priority,
        request_id: request_id.to_owned(),
        model: "gpt-4o".to_owned(),
    }
}

fn reader(port: u16) -> RoutingStateStore {
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(port)),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings::from(&config("model_list: []\n").sections.redis_failure_policy),
    };
    RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO)).expect("the reader builds")
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

// the config names no prompt caching check, so its cache and facts are never read
fn affinity_inputs() -> PromptAffinityInputs<LocalMemoryOps> {
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

// the pipeline reads Router.scheduler through the router services, and the config's interval reaches it
#[test]
fn the_router_services_expose_the_configured_scheduler() {
    let loaded = config("model_list: []\nrouter_settings:\n  polling_interval: 0.25\n");
    let services =
        RouterServices::from_config(&loaded, affinity_inputs()).expect("the router services build");
    let scheduler = services
        .request_scheduler()
        .expect("the scheduler is configured");
    assert_eq!(scheduler.polling_interval(), Duration::from_millis(250));
}

#[tokio::test]
async fn a_config_without_redis_builds_a_local_queue() {
    let scheduler = build(&config("model_list: []\n")).expect("the scheduler builds");
    scheduler
        .add_request(&item(0, "local"))
        .await
        .expect("the request queues");
    let queued = scheduler.queue("gpt-4o").await.expect("the queue reads");
    assert_eq!(queued.warning, None);
    assert_eq!(queued.value.len(), 1);
}

// the config's redis host and port reach the queue: an independent reader sees the entry the scheduler wrote
#[tokio::test]
async fn a_config_with_redis_writes_the_queue_the_shared_tier_holds() {
    let server = FakeRedis::try_start().await.expect("the fake redis starts");
    let scheduler = build(&redis_config(server.port())).expect("the scheduler builds");
    scheduler
        .add_request(&item(128, "shared"))
        .await
        .expect("the request queues");
    let read = reader(server.port())
        .get(&RouteKey::parse("scheduler:queue:gpt-4o").expect("the key"))
        .await
        .expect("the shared entry reads");
    assert_eq!(
        read.value,
        StoreRead::Value(RouteValue::Members(BTreeSet::from([
            "128:shared".to_owned()
        ])))
    );
}

// str(uuid.uuid4()) is the hyphenated 36-character form
#[test]
fn request_ids_are_uuids() {
    let generated = super::request_ids()();
    assert_eq!(generated.len(), 36);
    assert_eq!(generated.matches('-').count(), 4);
}
