pub mod counts;
pub mod float_increment;
pub mod floor;
pub mod guard;
pub mod increment;
pub mod reply;
pub mod scan;
pub mod scripts;
pub mod set_max;

mod api;
mod tests;

use std::result::Result as StdResult;
use std::sync::Arc;

use crate::cache_primitives::decode::DecodeError;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::transport::RedisTransport;
use crate::redis_live_client::LiveClientError;

pub type Counts = Vec<Option<i64>>;

pub type Result<T> = StdResult<T, RedisCountersError>;

pub struct RedisCounters {
    pub transport: Arc<RedisTransport>,
    pub namespace: Option<String>,
    pub default_ttl: i64,
    pub hooks: Arc<dyn ServiceHooks>,
    pub gate: Arc<dyn BreakerGate>,
}

#[derive(Debug, thiserror::Error)]
pub enum RedisCountersError {
    #[error(transparent)]
    Client(#[from] LiveClientError),
    #[error(transparent)]
    Reuse(#[from] ClientReuseError),
    #[error("a redis count could not be decoded")]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Breaker(#[from] BreakerOpen),
    #[error("redis returned an unexpected reply for {operation}")]
    UnexpectedReply { operation: &'static str },
    #[error("the ttl is not a finite number of seconds")]
    InvalidTtl,
    #[error("the value is not a finite number")]
    NonFiniteValue,
}
