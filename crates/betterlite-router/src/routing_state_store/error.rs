use betterlite_redis::redis_client_factories::error::FactoryError;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_connection_settings::error::RedisSettingsError;
use betterlite_redis::redis_counters::RedisCountersError;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::{ErrorKind, ServerErrorKind};
use thiserror::Error;

use crate::redis_failure_policy::BreakerEvent;
use crate::redis_failure_policy::classify::{Failure, FailureClass};
use crate::redis_failure_policy::run::RedisCallError;
use crate::routing_value_store::StoreError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StoreWarning {
    Unavailable {
        operation: &'static str,
        event: Option<BreakerEvent>,
    },
    Breaker(BreakerEvent),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Observed<T> {
    pub value: T,
    pub warning: Option<StoreWarning>,
}

#[derive(Debug, Error)]
pub enum StateStoreError {
    #[error(transparent)]
    Settings(#[from] RedisSettingsError),
    #[error(transparent)]
    Factory(#[from] FactoryError),
    #[error(transparent)]
    Reuse(#[from] ClientReuseError),
    #[error("redis routing state failed during {operation}")]
    Redis {
        operation: &'static str,
        #[source]
        source: LiveClientError,
    },
    #[error("redis routing state counters failed during {operation}")]
    Counters {
        operation: &'static str,
        #[source]
        source: RedisCountersError,
    },
    #[error("the redis circuit breaker is open — skipping {operation}")]
    CircuitOpen { operation: String },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the redis reply for {operation} could not be decoded")]
    Reply {
        operation: &'static str,
        #[source]
        source: redis::ParsingError,
    },
    #[error("the routing state ttl of {seconds} seconds is invalid")]
    InvalidTtl { seconds: f64 },
    #[error("the value under {key} kept changing during the update")]
    Conflict { key: String },
    #[error("atomic routing state changes need a standalone redis connection")]
    UnsupportedTarget,
    #[error("the routing state store has no dedicated redis connection")]
    NoConnection,
    #[error("no redis target is configured for routing state")]
    NotConfigured,
}

impl From<(&'static str, redis::RedisError)> for StateStoreError {
    fn from((operation, source): (&'static str, redis::RedisError)) -> Self {
        Self::Redis {
            operation,
            source: LiveClientError::Redis(source),
        }
    }
}

impl RedisCallError for StateStoreError {
    fn failure(&self) -> Option<Failure<'_>> {
        Some(Failure {
            class: failure_class(self)?,
            causes: &[],
        })
    }
}

fn failure_class(error: &StateStoreError) -> Option<FailureClass> {
    match error {
        StateStoreError::Redis { source, .. } => live_class(source),
        StateStoreError::Counters { source, .. } => match source {
            RedisCountersError::Client(client) => live_class(client),
            RedisCountersError::Reuse(reuse) => reuse_class(reuse),
            RedisCountersError::Decode(_)
            | RedisCountersError::Breaker(_)
            | RedisCountersError::UnexpectedReply { .. }
            | RedisCountersError::InvalidTtl
            | RedisCountersError::NonFiniteValue => None,
        },
        StateStoreError::Reuse(reuse) => reuse_class(reuse),
        StateStoreError::Settings(_)
        | StateStoreError::Factory(_)
        | StateStoreError::CircuitOpen { .. }
        | StateStoreError::Store(_)
        | StateStoreError::Reply { .. }
        | StateStoreError::InvalidTtl { .. }
        | StateStoreError::Conflict { .. }
        | StateStoreError::UnsupportedTarget
        | StateStoreError::NoConnection
        | StateStoreError::NotConfigured => None,
    }
}

fn reuse_class(error: &ClientReuseError) -> Option<FailureClass> {
    if matches!(error, ClientReuseError::Timeout(_)) {
        return Some(FailureClass::Timeout);
    }
    if let ClientReuseError::Live(source) = error {
        return live_class(source);
    }
    None
}

fn live_class(error: &LiveClientError) -> Option<FailureClass> {
    match error {
        LiveClientError::Redis(source) => redis_class(source),
        LiveClientError::NoSentinelMaster { .. } => Some(FailureClass::Connection),
        LiveClientError::Credential(_)
        | LiveClientError::CredentialSourceMissing
        | LiveClientError::InvalidSetting { .. }
        | LiveClientError::TlsFile { .. }
        | LiveClientError::Join(_) => None,
    }
}

fn redis_class(error: &redis::RedisError) -> Option<FailureClass> {
    if error.is_timeout() {
        return Some(FailureClass::Timeout);
    }
    if error.kind() == ErrorKind::Server(ServerErrorKind::BusyLoading) {
        return Some(FailureClass::BusyLoading);
    }
    if error.kind() == ErrorKind::Server(ServerErrorKind::ClusterDown) {
        return Some(FailureClass::ClusterDown);
    }
    (error.is_connection_dropped() || error.is_io_error()).then_some(FailureClass::Connection)
}
