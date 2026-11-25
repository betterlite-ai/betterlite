#![cfg(test)]

use std::fmt;
use std::io;
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::decode::DecodeError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::breaker::BreakerAdmission;
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate as _;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::RedisError;
use tokio::time::error::Elapsed;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::redis_failure_policy::gate::PolicyGate;
use crate::redis_failure_policy::{RedisFailurePolicy, Settings, Status};
use crate::routing_state_store::BreakerClock;

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

    pub(crate) fn handle(self: &Arc<Self>) -> BreakerClock {
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
    // a literal invalid byte trips the invalid_from_utf8 lint, so build the bytes at run time
    let mut bytes = vec![b'a'];
    bytes.push(0xff);
    let source = str::from_utf8(&bytes).expect_err("the bytes are not utf-8");
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
    RedisError::from((
        redis::ErrorKind::Extension,
        "the module replied with an error",
    ))
}

// redis builds this one from a description alone, so it has no cause for the source walk to reach
pub(crate) fn described_connection_error() -> RedisCacheError {
    wrapped(RedisError::from((
        redis::ErrorKind::Io,
        "connection dropped",
    )))
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
pub(crate) fn capture_logs<F>(run: F) -> Vec<LogLine>
where
    F: FnOnce(),
{
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(LogCapture(Arc::clone(&lines)));
    with_default(subscriber, run);
    lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[derive(Debug, thiserror::Error)]
#[error("the cache broke")]
pub(crate) struct Broken;

// A non transparent wrapper is the only way a test hands the classifier a redis or io error as a source link.
#[derive(Debug, thiserror::Error)]
#[error("the outer error")]
pub(crate) struct OuterRedis {
    #[source]
    pub(crate) source: RedisError,
}

#[derive(Debug, thiserror::Error)]
#[error("the outer error")]
pub(crate) struct OuterIo {
    #[source]
    pub(crate) source: io::Error,
}
