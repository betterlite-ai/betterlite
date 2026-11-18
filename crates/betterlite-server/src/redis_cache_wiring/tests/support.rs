#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use betterlite_logging::logging_service_events::types::ServiceEvent as LoggedServiceEvent;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::decode::DecodeError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::breaker::BreakerAdmission;
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientError;
use betterlite_router::redis_failure_policy::{RedisFailurePolicy, Settings, Status};
use chrono::{DateTime, FixedOffset};
use redis::{ErrorKind, RedisError};
use tokio::time::error::Elapsed;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::redis_cache_wiring::gate::{GateClock, PolicyGate};
use crate::redis_cache_wiring::hooks::ServiceEventHooks;

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

    pub(crate) fn handle(self: &Arc<Self>) -> GateClock {
        let clock = Arc::clone(self);
        Arc::new(move || Duration::from_millis(clock.millis.load(Ordering::Relaxed)))
    }

    pub(crate) fn advance(&self, millis: u64) {
        self.millis.fetch_add(millis, Ordering::Relaxed);
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

pub(crate) fn gate_with(settings: Settings, clock: &Arc<FakeClock>) -> PolicyGate {
    PolicyGate::new(
        RedisFailurePolicy::new(settings, BreakerMetrics::new(None)),
        clock.handle(),
    )
}

pub(crate) fn status(gate: &PolicyGate) -> Status {
    gate.policy
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .status()
}

// The cap test needs more swallowed calls than the set keeps, so the loop lives here, not in a test.
pub(crate) fn swallow_calls(
    gate: &PolicyGate,
    error: &RedisCacheError,
    count: usize,
) -> Vec<BreakerAdmission> {
    let mut admissions = Vec::new();
    for _ in 0..count {
        let admission = gate
            .enter("async_batch_get_cache")
            .expect("the breaker never opens");
        gate.swallowed(&admission, error);
        admissions.push(admission);
    }
    admissions
}

pub(crate) fn connection_error() -> RedisCacheError {
    wrapped(RedisError::from(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "Connection refused",
    )))
}

pub(crate) fn timeout_error() -> RedisCacheError {
    wrapped(RedisError::from(io::Error::from(io::ErrorKind::TimedOut)))
}

// redis-py reports a pool wait timeout as a ConnectionError chained from a TimeoutError.
pub(crate) fn chained_timeout_error() -> RedisCacheError {
    let timeout = io::Error::from(io::ErrorKind::TimedOut);
    wrapped(RedisError::from(io::Error::new(
        io::ErrorKind::ConnectionReset,
        timeout,
    )))
}

pub(crate) fn command_error() -> RedisCacheError {
    wrapped(reply_error(b"-ERR unknown command 'FOO'\r\n"))
}

pub(crate) fn busy_loading_error() -> RedisCacheError {
    wrapped(reply_error(
        b"-LOADING Redis is loading the dataset in memory\r\n",
    ))
}

pub(crate) fn cluster_down_error() -> RedisCacheError {
    wrapped(reply_error(b"-CLUSTERDOWN The cluster is down\r\n"))
}

pub(crate) fn decode_error() -> RedisCacheError {
    let source = std::str::from_utf8(&[0xff]).expect_err("the bytes are not utf-8");
    RedisCacheError::from(DecodeError::Utf8 { source })
}

pub(crate) fn connect_timeout_error(elapsed: Elapsed) -> RedisCacheError {
    RedisCacheError::from(ClientReuseError::Timeout(elapsed))
}

// the classifier reads a redis error handed over on its own, before any transparent wrapper hides it
pub(crate) fn bare_timeout_error() -> RedisError {
    RedisError::from(io::Error::from(io::ErrorKind::TimedOut))
}

pub(crate) fn bare_connection_error() -> RedisError {
    RedisError::from(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "Connection refused",
    ))
}

// redis reports a reply code it doesn't know as an extension error
pub(crate) fn extension_error() -> RedisError {
    RedisError::from((ErrorKind::Extension, "the module replied with an error"))
}

// redis builds this one from a description alone, so it has no cause for the source walk to reach
pub(crate) fn described_connection_error() -> RedisCacheError {
    wrapped(RedisError::from((ErrorKind::Io, "connection dropped")))
}

pub(crate) fn bare_command_error() -> RedisError {
    reply_error(b"-ERR unknown command 'FOO'\r\n")
}

pub(crate) fn bare_busy_loading_error() -> RedisError {
    reply_error(b"-LOADING Redis is loading the dataset in memory\r\n")
}

pub(crate) fn bare_cluster_down_error() -> RedisError {
    reply_error(b"-CLUSTERDOWN The cluster is down\r\n")
}

pub(crate) fn master_down_error() -> RedisCacheError {
    wrapped(reply_error(b"-MASTERDOWN Link with MASTER is down\r\n"))
}

pub(crate) fn no_auth_error() -> RedisCacheError {
    wrapped(reply_error(b"-NOAUTH Authentication required.\r\n"))
}

pub(crate) fn max_clients_error() -> RedisCacheError {
    wrapped(reply_error(b"-ERR max number of clients reached\r\n"))
}

pub(crate) fn bare_master_down_error() -> RedisError {
    reply_error(b"-MASTERDOWN Link with MASTER is down\r\n")
}

pub(crate) fn bare_no_auth_error() -> RedisError {
    reply_error(b"-NOAUTH Authentication required.\r\n")
}

pub(crate) fn bare_max_clients_error() -> RedisError {
    reply_error(b"-ERR max number of clients reached\r\n")
}

fn wrapped(error: RedisError) -> RedisCacheError {
    RedisCacheError::from(ClientReuseError::from(LiveClientError::from(error)))
}

pub(crate) fn reply_error(reply: &[u8]) -> RedisError {
    redis::parse_redis_value(reply)
        .expect("the reply parses")
        .extract_error()
        .expect_err("the reply is an error")
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
pub(crate) fn capture_logs<F: FnOnce()>(run: F) -> Vec<LogLine> {
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
        let guard = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        let (events, _timed_out) = self
            .arrived
            .wait_timeout_while(guard, SINK_WAIT, |pending| pending.len() < count)
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

// A listener on port zero tells us a port nothing listens on once it is closed again.
pub(crate) fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let port = listener.local_addr().expect("the listener address").port();
    drop(listener);
    port
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
