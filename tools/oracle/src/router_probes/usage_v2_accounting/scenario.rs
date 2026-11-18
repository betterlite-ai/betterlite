use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_config::sections::routing_state as state_config;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_config::sections::usage_v2_accounting as config;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::routing_state::counter::RouterCounters;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings as RedisStoreSettings,
};
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};
use betterlite_router::usage_v2_accounting::UsageV2Accounting;
use betterlite_server::routing::usage_v2_accounting as adapter;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod steps;

const NAMESPACE: &str = "b371";
const FAILURE_THRESHOLD: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    pub ttl_seconds: Option<u64>,
    pub steps: Vec<ScenarioStep>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub kind: StepKind,
    #[serde(default)]
    pub now_millis: u64,
    #[serde(default)]
    pub sync: bool,
    pub deployment: Option<DeploymentInput>,
    pub call_type: Option<String>,
    pub standard_logging_object: Option<StandardLoggingInput>,
    pub key: Option<String>,
    pub value: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    PreCall,
    Success,
    Sync,
    SeedStore,
    Store,
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub model_id: Option<String>,
    pub model: Option<String>,
    pub rpm: Option<i64>,
    pub params_rpm: Option<i64>,
    pub model_info_rpm: Option<i64>,
    pub num_retries: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLoggingInput {
    pub model_group: Option<String>,
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioOutput {
    pub steps: Vec<Observation>,
}

// one entry per step: exactly the value the step made observable, tagged so every answer keeps its type
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Allowed {
        step: usize,
    },
    Rejected {
        step: usize,
        status_code: u16,
        code: String,
        error_type: String,
        retry_after: Option<String>,
        num_retries: Option<i64>,
    },
    Recorded {
        step: usize,
    },
    Store {
        step: usize,
        value: String,
    },
}

// the adapter's own failures, each holding the error it came from until the probe boundary renders it
#[derive(Debug, Error)]
pub(super) enum ScenarioError {
    #[error("the fake redis server did not start")]
    Redis {
        #[source]
        source: io::Error,
    },
    #[error("the routing store failed")]
    Store {
        #[from]
        source: StateStoreError,
    },
    #[error("the in-memory value store is invalid")]
    Local {
        #[from]
        source: StoreError,
    },
    #[error("a routing key is invalid")]
    Key {
        #[source]
        source: StoreError,
    },
    #[error("the step needs {what}")]
    Missing { what: &'static str },
    #[error("the stored routing value is not finite")]
    NonFinite,
    #[error("the step time {millis} is not representable")]
    Time { millis: u64 },
}

struct Context<'run> {
    accounting: &'run UsageV2Accounting<RoutingStateStore>,
    server: &'run FakeRedis,
    store: &'run RoutingStateStore,
}

pub(super) async fn run_scenario(input: &ScenarioInput) -> Result<ScenarioOutput, ScenarioError> {
    let server = FakeRedis::try_start()
        .await
        .map_err(|source| ScenarioError::Redis { source })?;
    let clock = Arc::new(AtomicU64::new(0));
    let section = config::Settings {
        ttl: Duration::from_secs(input.ttl_seconds.unwrap_or(config::DEFAULT_TTL_SECONDS)),
    };
    let adapter::Resolved {
        ttl,
        counters: counter_settings,
    } = adapter::resolve(&section, &state_config::Settings::default());
    let store = Arc::new(RoutingStateStore::new(
        &redis_settings(&server),
        breaker_clock(&clock),
    )?);
    let local = RoutingValueStore::new(local_settings(), value_clock(&clock))?;
    let counters = RouterCounters::new(counter_settings, Arc::new(local), Some(Arc::clone(&store)));
    let accounting = UsageV2Accounting {
        counters: Arc::new(counters),
        ttl,
    };
    let context = Context {
        accounting: &accounting,
        server: &server,
        store: store.as_ref(),
    };
    let mut observations = Vec::with_capacity(input.steps.len());
    for (index, step) in input.steps.iter().enumerate() {
        clock.store(step.now_millis, Ordering::SeqCst);
        observations.push(steps::apply(index, step, &context).await?);
    }
    Ok(ScenarioOutput {
        steps: observations,
    })
}

fn redis_settings(server: &FakeRedis) -> RedisStoreSettings {
    RedisStoreSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: Some(NAMESPACE.to_owned()),
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: FAILURE_THRESHOLD,
            recovery_timeout: Duration::ZERO,
            timeout_min_duration: Duration::from_millis(5_000),
            timeout_log_interval: Duration::from_millis(1_000),
        },
    }
}

fn value_clock(clock: &Arc<AtomicU64>) -> Clock {
    let source = Arc::clone(clock);
    let read = move || Duration::from_millis(source.load(Ordering::SeqCst)).as_secs_f64();
    Arc::new(read)
}

fn breaker_clock(clock: &Arc<AtomicU64>) -> BreakerClock {
    let source = Arc::clone(clock);
    let read = move || Duration::from_millis(source.load(Ordering::SeqCst));
    Arc::new(read)
}

const fn local_settings() -> StoreSettings {
    StoreSettings {
        max_size_in_memory: Some(200),
        default_ttl_seconds: None,
        max_size_per_item: None,
        loaded_max_size_per_item_kib: 1024,
    }
}
