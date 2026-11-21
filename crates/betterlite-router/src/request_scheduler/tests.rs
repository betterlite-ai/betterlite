#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::request_scheduler::Settings;
use betterlite_config::sections::routing_state_store::connection::Connection;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::request_scheduler::flow::FlowItem;
use crate::request_scheduler::schedule::{ScheduledCall, ScheduledFailure};
use crate::request_scheduler::service::{RequestIdSource, RequestScheduler};
use crate::routing_state_store::Settings as SharedSettings;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{RedisSettings, RoutingStateStore};
use crate::routing_timeouts::global::GlobalRequestTimeout;
use crate::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, TimeoutPlan, resolve_router_timeouts,
};
use crate::routing_timeouts::seconds::Seconds;
use crate::routing_value_store::{RoutingValueStore, SchedulerTuple, StoreSettings};

mod request_scheduler_behaviors;
mod request_scheduler_boundaries;

pub(super) const MODEL: &str = "gpt-4o";

pub(super) fn local_store() -> RoutingValueStore {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(|| 0.0_f64),
    )
    .expect("the local store builds")
}

fn request_ids() -> RequestIdSource {
    Arc::new(|| "generated".to_owned())
}

pub(super) fn scheduler(settings: Settings) -> RequestScheduler {
    RequestScheduler::new(local_store(), None, settings, request_ids())
}

// the shared tier the pin attaches when Redis is configured, with the same defaults the server builds
pub(super) async fn shared_scheduler() -> (RequestScheduler, FakeRedis) {
    let server = FakeRedis::try_start().await.expect("the fake redis starts");
    let settings = SharedSettings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 0.25,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 100,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    let shared = RoutingStateStore::new(&settings, Arc::new(|| Duration::ZERO))
        .expect("the shared store builds");
    let scheduler = RequestScheduler::new(
        local_store(),
        Some(shared),
        Settings::default(),
        request_ids(),
    );
    (scheduler, server)
}

pub(super) fn item(priority: u8, request_id: &str, model: &str) -> FlowItem {
    FlowItem {
        priority,
        request_id: request_id.to_owned(),
        model: model.to_owned(),
    }
}

pub(super) fn entry(priority: u8, request_id: &str) -> SchedulerTuple {
    SchedulerTuple {
        priority,
        request_id: request_id.to_owned(),
    }
}

pub(super) fn entries(pairs: &[(u8, &str)]) -> Vec<SchedulerTuple> {
    pairs
        .iter()
        .map(|(priority, request_id)| entry(*priority, request_id))
        .collect()
}

// Router(timeout=seconds) over the paused clock the scheduling tests run on
pub(super) fn plan(seconds: f64) -> TimeoutPlan {
    let sources = RouterTimeoutSources {
        timeout: Some(Seconds::new("router_settings.timeout", seconds).expect("a finite timeout")),
        stream_timeout: None,
        default_stream_timeout: None,
    };
    let global = GlobalRequestTimeout {
        seconds: Seconds(6000.0_f64),
        explicitly_set: false,
    };
    let request = RequestTimeouts {
        stream: false,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    };
    TimeoutPlan::new(
        Instant::now().into_std(),
        &resolve_router_timeouts(&sources, global),
        &request,
        &DeploymentTimeouts::default(),
    )
    .expect("the plan builds")
}

pub(super) struct ScriptedCall {
    healthy: usize,
    answer: Result<(), ScheduledFailure>,
    // the caller going away while the request waits, cancelled at its first poll
    cancel: Option<CancellationToken>,
    calls: AtomicUsize,
}

impl ScriptedCall {
    pub(super) fn answered(healthy: usize) -> Self {
        Self {
            healthy,
            answer: Ok(()),
            cancel: None,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn failed(healthy: usize, status: u16, code: Option<&str>) -> Self {
        Self {
            healthy,
            answer: Err(ScheduledFailure {
                status,
                code: code.map(str::to_owned),
                priority: 0,
            }),
            cancel: None,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn cancelling(healthy: usize, cancel: CancellationToken) -> Self {
        Self {
            healthy,
            answer: Ok(()),
            cancel: Some(cancel),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ScheduledCall for ScriptedCall {
    fn healthy_deployments(&self) -> usize {
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
        self.healthy
    }

    async fn call(&self) -> Result<(), ScheduledFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}
