#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use betterlite_logging::logging_service_events::types::ServiceEvent as LoggedServiceEvent;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::decode::DecodeError;
use betterlite_redis::redis_cache::error::RedisCacheError;
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

use crate::redis_cache_wiring::gate::{GateClock, PolicyGate};

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const TIMEOUT_MIN_DURATION: Duration = Duration::from_secs(60);
const TIMEOUT_LOG_INTERVAL: Duration = Duration::from_secs(60);

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
    PolicyGate {
        policy: Mutex::new(RedisFailurePolicy::new(settings, BreakerMetrics::new(None))),
        next_call: AtomicU64::new(0),
        swallowed_calls: Mutex::new(BTreeSet::new()),
        clock: clock.handle(),
    }
}

pub(crate) fn status(gate: &PolicyGate) -> Status {
    gate.policy
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .status()
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

fn wrapped(error: RedisError) -> RedisCacheError {
    RedisCacheError::from(ClientReuseError::from(LiveClientError::from(error)))
}

fn reply_error(reply: &[u8]) -> RedisError {
    redis::parse_redis_value(reply)
        .expect("the reply parses")
        .extract_error()
        .expect_err("the reply is an error")
}

pub(crate) struct RecordingSink {
    events: Mutex<Vec<LoggedServiceEvent>>,
}

impl RecordingSink {
    pub(crate) fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn events(&self) -> Vec<LoggedServiceEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ServiceEventSink for RecordingSink {
    fn emit(&self, event: &LoggedServiceEvent) -> Result<(), ServiceEventError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
        Ok(())
    }
}

pub(crate) struct FailingSink;

impl ServiceEventSink for FailingSink {
    fn emit(&self, _event: &LoggedServiceEvent) -> Result<(), ServiceEventError> {
        Err(ServiceEventError::Rejected {
            message: "the sink is down".to_owned(),
        })
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

#[derive(Debug)]
pub(crate) struct Broken;

impl fmt::Display for Broken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the cache broke")
    }
}

impl StdError for Broken {}

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
        socket_timeout: Some(FloatOrText::Value(1.0)),
        socket_connect_timeout: Some(FloatOrText::Value(1.0)),
        ..RedisOverrides::default()
    }
}
