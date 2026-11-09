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
    #[error("the redis reply for {operation} was not a value this store understands")]
    UnexpectedReply { operation: &'static str },
    #[error("the redis reply for {operation} could not be decoded")]
    Reply {
        operation: &'static str,
        #[source]
        source: redis::ParsingError,
    },
    #[error("the routing state store needs redis for {operation}")]
    RedisRequired { operation: &'static str },
    #[error("the routing state ttl of {seconds} seconds is invalid")]
    InvalidTtl { seconds: f64 },
    #[error("the routing value under {key} could not be encoded")]
    Encoding {
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the value under {key} kept changing during the update")]
    Conflict { key: String },
    #[error("atomic routing state changes need a standalone redis connection")]
    UnsupportedTarget,
    #[error("the routing state store has no dedicated redis connection")]
    NoConnection,
}

impl RedisCallError for StateStoreError {
    fn failure(&self) -> Option<Failure<'_>> {
        let class = match self {
            Self::Redis { source, .. } => live_failure(source),
            Self::Reuse(source) => reuse_failure(source),
            Self::Counters { source, .. } => counters_failure(source),
            Self::Settings(_)
            | Self::Factory(_)
            | Self::CircuitOpen { .. }
            | Self::Store(_)
            | Self::UnexpectedReply { .. }
            | Self::Reply { .. }
            | Self::RedisRequired { .. }
            | Self::InvalidTtl { .. }
            | Self::Encoding { .. }
            | Self::Conflict { .. }
            | Self::UnsupportedTarget
            | Self::NoConnection => None,
        }?;
        Some(Failure { class, causes: &[] })
    }
}

fn live_failure(error: &LiveClientError) -> Option<FailureClass> {
    match error {
        LiveClientError::Redis(source) => redis_failure(source),
        LiveClientError::NoSentinelMaster { .. } => Some(FailureClass::Connection),
        LiveClientError::Credential(_)
        | LiveClientError::CredentialSourceMissing
        | LiveClientError::InvalidSetting { .. }
        | LiveClientError::TlsFile { .. }
        | LiveClientError::Join(_) => None,
    }
}

fn reuse_failure(error: &ClientReuseError) -> Option<FailureClass> {
    match error {
        ClientReuseError::Live(source) => live_failure(source),
        ClientReuseError::Factory(_)
        | ClientReuseError::NoPubSubNode
        | ClientReuseError::ClusterPubSub => None,
        ClientReuseError::Timeout(_) => Some(FailureClass::Timeout),
    }
}

fn counters_failure(error: &RedisCountersError) -> Option<FailureClass> {
    match error {
        RedisCountersError::Client(source) => live_failure(source),
        RedisCountersError::Reuse(source) => reuse_failure(source),
        RedisCountersError::Decode(_)
        | RedisCountersError::Breaker(_)
        | RedisCountersError::UnexpectedReply { .. }
        | RedisCountersError::InvalidTtl
        | RedisCountersError::NonFiniteValue => None,
    }
}

fn redis_failure(error: &redis::RedisError) -> Option<FailureClass> {
    if error.is_timeout() {
        return Some(FailureClass::Timeout);
    }
    if error.kind() == ErrorKind::Server(ServerErrorKind::BusyLoading) {
        return Some(FailureClass::BusyLoading);
    }
    if error.kind() == ErrorKind::Server(ServerErrorKind::ClusterDown) {
        return Some(FailureClass::ClusterDown);
    }
    if error.is_connection_dropped() || error.is_io_error() {
        return Some(FailureClass::Connection);
    }
    None
}
