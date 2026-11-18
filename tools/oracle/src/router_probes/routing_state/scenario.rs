use std::collections::BTreeMap;
use std::io;
use std::num::TryFromIntError;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::routing_state as sync_config;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::router_observations::counter::CounterError;
use betterlite_router::routing_state::Settings as RouterSettings;
use betterlite_router::routing_state::counter::RouterCounters;
use betterlite_router::routing_state::error::RoutingStateError;
use betterlite_router::routing_state_store::error::StateStoreError;
use betterlite_router::routing_state_store::test_support::FakeRedis;
use betterlite_router::routing_state_store::{
    BreakerClock, RedisSettings, RoutingStateStore, Settings as RedisStoreSettings,
};
use betterlite_router::routing_value_store::{Clock, RoutingValueStore, StoreError, StoreSettings};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod counters;
mod lifecycle;

const TTL_SECONDS: u64 = 60;
const NAMESPACE: &str = "b304";
const FAILURE_THRESHOLD: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioInput {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub steps: Vec<ScenarioStep>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioStep {
    pub kind: StepKind,
    #[serde(default)]
    pub now_millis: u64,
    pub deployment_id: Option<String>,
    pub model: Option<String>,
    pub repeat: Option<usize>,
    pub sync_interval_millis: Option<u64>,
    pub key: Option<String>,
    pub channels: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    PreCall,
    Success,
    Failure,
    Outcomes,
    Tracked,
    Sync,
    Retire,
    Down,
    Up,
    Resolve,
    Metadata,
    Redis,
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
    Recorded {
        step: usize,
    },
    Key {
        step: usize,
        key: String,
    },
    Keys {
        step: usize,
        keys: Vec<String>,
    },
    Outcomes {
        step: usize,
        successes: i64,
        failures: i64,
    },
    Millis {
        step: usize,
        millis: u64,
    },
    Channel {
        step: usize,
        channel: Option<String>,
        tokens: Option<i64>,
    },
    Redis {
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
    #[error("the routing state section is invalid")]
    Config {
        #[from]
        source: ConfigError,
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
    #[error("a routing window increment failed")]
    Increment {
        #[from]
        source: RoutingStateError,
    },
    #[error("a routing outcome counter failed")]
    Counter {
        #[from]
        source: CounterError,
    },
    #[error("a routing key is invalid")]
    Key {
        #[source]
        source: StoreError,
    },
    #[error("the channels object is invalid")]
    Channels {
        #[from]
        source: serde_json::Error,
    },
    #[error("the step needs {what}")]
    Missing { what: &'static str },
    #[error("the stored routing value is not finite")]
    NonFinite,
    #[error("the step duration does not fit the millis output")]
    Duration {
        #[from]
        source: TryFromIntError,
    },
}

struct Context<'run> {
    counters: &'run RouterCounters<RoutingStateStore>,
    server: &'run FakeRedis,
    store: &'run RoutingStateStore,
    settings: &'run RouterSettings,
}

pub(super) async fn run_scenario(input: &ScenarioInput) -> Result<ScenarioOutput, ScenarioError> {
    let server = FakeRedis::try_start()
        .await
        .map_err(|source| ScenarioError::Redis { source })?;
    let clock = Arc::new(AtomicU64::new(0));
    let section = sync_config::Raw::default().convert(&input.env)?;
    let settings = RouterSettings {
        sync_interval: None,
        default_sync_interval: section.default_sync_interval,
        should_batch_redis_writes: true,
    };
    let store = Arc::new(RoutingStateStore::new(
        &redis_settings(&server),
        breaker_clock(&clock),
    )?);
    let local = RoutingValueStore::new(local_settings(), value_clock(&clock))?;
    let counters = RouterCounters::new(settings.clone(), Arc::new(local), Some(Arc::clone(&store)));
    let context = Context {
        counters: &counters,
        server: &server,
        store: store.as_ref(),
        settings: &settings,
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for (index, step) in input.steps.iter().enumerate() {
        clock.store(step.now_millis, Ordering::SeqCst);
        steps.push(apply(index, step, &context).await?);
    }
    Ok(ScenarioOutput { steps })
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

async fn apply(
    index: usize,
    step: &ScenarioStep,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    match step.kind {
        StepKind::PreCall => counters::pre_call(index, step, context).await,
        StepKind::Success => counters::outcome(index, step, context, counters::Outcome::Success),
        StepKind::Failure => counters::outcome(index, step, context, counters::Outcome::Failure),
        // track_deployment_metrics.get_deployment_successes_for_current_minute / failures
        StepKind::Outcomes => {
            let deployment = step
                .deployment_id
                .as_deref()
                .ok_or(ScenarioError::Missing {
                    what: "a deployment id",
                })?;
            Ok(Observation::Outcomes {
                step: index,
                successes: context.counters.successes_for_minute(deployment),
                failures: context.counters.failures_for_minute(deployment),
            })
        }
        StepKind::Tracked => Ok(counters::tracked(index, context)),
        StepKind::Sync => lifecycle::sync(index, context).await,
        StepKind::Retire => lifecycle::retire(index, context).await,
        StepKind::Down => Ok(lifecycle::switch(index, context, true)),
        StepKind::Up => Ok(lifecycle::switch(index, context, false)),
        StepKind::Resolve => resolved_interval(index, step, context.settings.default_sync_interval),
        StepKind::Metadata => lifecycle::metadata(index, step),
        StepKind::Redis => lifecycle::redis(index, step, context).await,
    }
}

// the step's own interval wins, then the configured default, then LiteLLM's constant
fn resolved_interval(
    index: usize,
    step: &ScenarioStep,
    default_sync_interval: Duration,
) -> Result<Observation, ScenarioError> {
    let settings = RouterSettings {
        sync_interval: step.sync_interval_millis.map(Duration::from_millis),
        default_sync_interval,
        should_batch_redis_writes: false,
    };
    let millis = u64::try_from(settings.resolved_sync_interval().as_millis())?;
    Ok(Observation::Millis {
        step: index,
        millis,
    })
}
