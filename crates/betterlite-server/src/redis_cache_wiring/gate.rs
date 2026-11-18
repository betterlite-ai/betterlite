use std::collections::BTreeSet;
use std::error::Error as StdError;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_router::redis_failure_policy::classify::{
    Failure, FailureClass, MAX_CAUSE_DEPTH, is_health_failure,
};
use betterlite_router::redis_failure_policy::{RedisFailurePolicy, Settings};
use redis::{ErrorKind, RedisError, ServerError, ServerErrorKind};
use tokio::time::error::Elapsed;

use crate::routing::redis_failure_policy::{FailureLogLevel, log_breaker_event, log_redis_failure};

// The gate reads the clock through this, so a test can move the breaker past its recovery timeout.
pub type GateClock = Arc<dyn Fn() -> Duration + Send + Sync>;

// An admission is the generation times 2^32 plus the call number, and neither reaches 2^32 in a process.
const CALL_SPACE: u64 = 0x1_0000_0000;

// A call that swallows a failure and never exits leaves its number here until an exit or the cap drops it.
pub(crate) const SWALLOWED_CALLS_KEPT: usize = 4096;

// One breaker per cache, as LiteLLM gives every RedisCache instance its own RedisCircuitBreaker.
pub struct PolicyGate {
    pub policy: Mutex<RedisFailurePolicy>,
    pub next_call: AtomicU64,
    pub swallowed_calls: Mutex<BTreeSet<u64>>,
    pub clock: GateClock,
}

impl BreakerGate for PolicyGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        let now = (self.clock)();
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        if policy.admit(now, name).is_err() {
            return Err(BreakerOpen { name });
        }
        let generation = policy.status().generation;
        drop(policy);
        let call = self.next_call.fetch_add(1, Ordering::Relaxed);
        Ok(BreakerAdmission::from(
            generation
                .wrapping_mul(CALL_SPACE)
                .wrapping_add(call.rem_euclid(CALL_SPACE)),
        ))
    }

    fn exit(&self, admission: BreakerAdmission) {
        let bits = *admission.as_ref();
        let generation = bits.div_euclid(CALL_SPACE);
        let call = bits.rem_euclid(CALL_SPACE);
        let swallowed = self
            .swallowed_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&call);
        if swallowed {
            return;
        }
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        if policy.status().generation != generation {
            return;
        }
        if let Some(event) = policy.record_success() {
            log_breaker_event(&event);
        }
    }

    fn record_failure(&self, error: &(dyn StdError + Send + Sync)) {
        let Some((class, causes)) = classify(error) else {
            return;
        };
        if !is_health_failure(class) {
            return;
        }
        self.record(
            Failure {
                class,
                causes: &causes,
            },
            error,
        );
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn StdError + Send + Sync)) {
        let Some((class, causes)) = classify(error) else {
            return;
        };
        if !is_health_failure(class) {
            return;
        }
        self.record(
            Failure {
                class,
                causes: &causes,
            },
            error,
        );
        let call = admission.as_ref().rem_euclid(CALL_SPACE);
        let mut kept = self
            .swallowed_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        kept.insert(call);
        let excess = kept.len().saturating_sub(SWALLOWED_CALLS_KEPT);
        for _ in 0..excess {
            let _dropped = kept.pop_first();
        }
    }
}

impl PolicyGate {
    // LiteLLM's health filter sees the exception it caught, so the outermost class decides; a deeper
    // cause only marks a connection failure as a timeout, the way a chained TimeoutError does.
    fn record(&self, failure: Failure<'_>, error: &(dyn StdError + Send + Sync)) {
        let now = (self.clock)();
        let mut policy = self.policy.lock().unwrap_or_else(PoisonError::into_inner);
        let event = policy.record_failure(now, failure);
        let decision = policy.log_failure(now, failure);
        drop(policy);
        if let Some(opened) = event {
            log_breaker_event(&opened);
        }
        log_redis_failure(
            decision,
            FailureLogLevel::Error,
            "Redis operation failed",
            &error.to_string(),
        );
    }
}

pub(crate) fn classify(
    error: &(dyn StdError + Send + Sync),
) -> Option<(FailureClass, Vec<FailureClass>)> {
    let root: &dyn StdError = error;
    let mut current: Option<&dyn StdError> = Some(root);
    let mut classes = Vec::new();
    for _ in 0..MAX_CAUSE_DEPTH {
        let Some(link) = current else {
            break;
        };
        if let Some(class) = class_of(link) {
            classes.push(class);
        }
        current = link.source();
    }
    let mut remaining = classes.into_iter();
    let class = remaining.next()?;
    Some((class, remaining.collect()))
}

fn class_of(error: &(dyn StdError + 'static)) -> Option<FailureClass> {
    if let Some(redis) = error.downcast_ref::<RedisError>() {
        return Some(redis_class(redis));
    }
    if let Some(server) = error.downcast_ref::<ServerError>() {
        return Some(server_class(server));
    }
    if let Some(io) = error.downcast_ref::<io::Error>() {
        return Some(io_class(io));
    }
    if error.downcast_ref::<Elapsed>().is_some() {
        return Some(FailureClass::AsynchronousTimeout);
    }
    None
}

// A timeout is an io error inside the RedisError, and the walk reaches that io error next, so the
// timeout class comes from the cause rather than a second is_timeout branch here.
fn redis_class(error: &RedisError) -> FailureClass {
    match error.kind() {
        ErrorKind::Server(ServerErrorKind::BusyLoading) => FailureClass::BusyLoading,
        ErrorKind::Server(ServerErrorKind::ClusterDown) => FailureClass::ClusterDown,
        ErrorKind::Io => FailureClass::Connection,
        ErrorKind::Server(_) | ErrorKind::Extension => FailureClass::Command,
        _ => FailureClass::Other,
    }
}

fn server_class(error: &ServerError) -> FailureClass {
    match error.kind() {
        Some(ServerErrorKind::BusyLoading) => FailureClass::BusyLoading,
        Some(ServerErrorKind::ClusterDown) => FailureClass::ClusterDown,
        Some(_) | None => FailureClass::Command,
    }
}

fn io_class(error: &io::Error) -> FailureClass {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => FailureClass::Timeout,
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::NotConnected
        | io::ErrorKind::NotFound
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::NetworkDown => FailureClass::Connection,
        _ => FailureClass::Os,
    }
}
