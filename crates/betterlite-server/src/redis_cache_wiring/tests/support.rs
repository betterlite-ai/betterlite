#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use crate::redis_cache_wiring::hooks::ServiceEventHooks;
use betterlite_logging::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use betterlite_logging::logging_service_events::types::ServiceEvent as LoggedServiceEvent;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientError;
use betterlite_router::redis_failure_policy::Settings;
use betterlite_router::routing_state_store::BreakerClock;
use chrono::{DateTime, FixedOffset};
use redis::RedisError;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const TIMEOUT_MIN_DURATION: Duration = Duration::from_secs(60);
const TIMEOUT_LOG_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const SINK_WAIT: Duration = Duration::from_secs(10);

pub(crate) struct FakeClock {
    millis: AtomicU64,
}

impl FakeClock {
    pub(crate) fn new() -> Self {
        Self {
            millis: AtomicU64::new(0),
        }
    }

    pub(crate) fn handle(self: &Arc<Self>) -> BreakerClock {
        let clock = Arc::clone(self);
        Arc::new(move || Duration::from_millis(clock.millis.load(Ordering::Relaxed)))
    }
}

pub(crate) fn settings(failure_threshold: u32) -> Settings {
    Settings {
        enabled: true,
        failure_threshold,
        recovery_timeout: RECOVERY_TIMEOUT,
        timeout_min_duration: TIMEOUT_MIN_DURATION,
        timeout_log_interval: TIMEOUT_LOG_INTERVAL,
    }
}

pub(crate) fn connection_error() -> RedisCacheError {
    RedisCacheError::from(ClientReuseError::from(LiveClientError::from(
        RedisError::from(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "Connection refused",
        )),
    )))
}

#[derive(Clone)]
pub(crate) struct LogLine {
    pub(crate) level: Level,
    pub(crate) message: String,
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct LogCapture(Arc<Mutex<Vec<LogLine>>>);

impl<S: Subscriber> Layer<S> for LogCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LogLine {
                level: *event.metadata().level(),
                message: visitor.0,
            });
    }
}

// A test installs this on its own thread and reads back the lines the code under test writes.
pub(crate) fn capture_logs<F>(run: F) -> Vec<LogLine>
where
    F: FnOnce(),
{
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(LogCapture(Arc::clone(&lines)));
    with_default(subscriber, run);
    lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(crate) struct RecordingSink {
    events: Mutex<Vec<LoggedServiceEvent>>,
    arrived: Condvar,
}

impl RecordingSink {
    pub(crate) fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            arrived: Condvar::new(),
        }
    }

    // the worker thread delivers asynchronously, so a test waits for the events instead of reading once
    pub(crate) fn wait_for_events(&self, count: usize) -> Vec<LoggedServiceEvent> {
        let (events, _timed_out) = self
            .arrived
            .wait_timeout_while(
                self.events.lock().unwrap_or_else(PoisonError::into_inner),
                SINK_WAIT,
                |pending| pending.len() < count,
            )
            .unwrap_or_else(PoisonError::into_inner);
        events.clone()
    }
}

impl ServiceEventSink for RecordingSink {
    fn emit(&self, event: &LoggedServiceEvent) -> Result<(), ServiceEventError> {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        events.push(event.clone());
        drop(events);
        self.arrived.notify_all();
        Ok(())
    }
}

pub(crate) struct FailingSink {
    pub(crate) calls: SyncSender<()>,
}

impl ServiceEventSink for FailingSink {
    fn emit(&self, _event: &LoggedServiceEvent) -> Result<(), ServiceEventError> {
        let _signalled = self.calls.try_send(());
        Err(ServiceEventError::Rejected {
            message: "the sink is down".to_owned(),
        })
    }
}

// The first emit blocks until the test releases it, so a hook that called the sink inline would hang too.
pub(crate) struct BlockedSink {
    entered: SyncSender<()>,
    release: Mutex<Receiver<()>>,
    first: AtomicBool,
}

impl BlockedSink {
    pub(crate) fn new(entered: SyncSender<()>, release: Receiver<()>) -> Self {
        Self {
            entered,
            release: Mutex::new(release),
            first: AtomicBool::new(false),
        }
    }
}

impl ServiceEventSink for BlockedSink {
    fn emit(&self, _event: &LoggedServiceEvent) -> Result<(), ServiceEventError> {
        if !self.first.swap(true, Ordering::SeqCst) {
            let _announced = self.entered.try_send(());
            let release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
            let _released = release.recv();
        }
        Ok(())
    }
}

// The queue test sends more events than the queue holds, so the loop lives here, not in a test.
pub(crate) fn emit_successes(hooks: &ServiceEventHooks, event: &ServiceEvent, count: usize) {
    for _ in 0..count {
        hooks.success(event);
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<FixedOffset>,
}

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

pub(crate) fn fixed_clock() -> FixedClock {
    FixedClock {
        now: DateTime::parse_from_rfc3339("2026-01-01T12:34:56+00:00").expect("a fixed timestamp"),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the cache broke")]
pub(crate) struct Broken;

pub(crate) struct NoopHooks;

impl ServiceHooks for NoopHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

pub(crate) fn empty_environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(crate) fn overrides(port: u16) -> RedisOverrides {
    RedisOverrides {
        host: Some("127.0.0.1".to_owned()),
        port: Some(IntOrText::Value(i64::from(port))),
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: Some(FloatOrText::Value(1.0)),
        socket_connect_timeout: Some(FloatOrText::Value(1.0)),
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        gcp_service_account: None,
        gcp_ssl_ca_certs: None,
        azure_redis_ad_token: None,
        azure_client_id: None,
        azure_tenant_id: None,
        azure_client_secret: None,
        aws_iam_auth: None,
        aws_iam_user_name: None,
        aws_iam_cache_name: None,
        aws_iam_region: None,
        aws_iam_serverless: None,
        passthrough: BTreeMap::new(),
    }
}
