use redis::ParsingError;
use thiserror::Error;

use crate::cache_primitives::decode::DecodeError;
use crate::redis_cache::error::RedisCacheError;
use crate::redis_cache_admin::RedisAdminError;
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, Error)]
pub enum BatchError {
    #[error("the batch pipeline failed: {0}")]
    Pipeline(#[source] PipelineFailure),
    #[error("a redis reply for a pipelined operation failed: {0}")]
    Reply(#[source] LiveClientError),
    #[error("the batch pipeline returned no reply for {operation}")]
    MissingReply { operation: &'static str },
    #[error("the MGET reply is {kind}, not an array")]
    MgetReply {
        kind: &'static str,
        #[source]
        source: ParsingError,
    },
    #[error("an MGET value is {kind}, not a bulk string or nil")]
    MgetValue {
        kind: &'static str,
        #[source]
        source: ParsingError,
    },
    #[error("the INCRBYFLOAT reply is {kind}, not a number")]
    IncrementReply {
        kind: &'static str,
        #[source]
        source: ParsingError,
    },
    #[error("the batch get asked for {requested} keys and returned {returned}")]
    MissingKeys { requested: usize, returned: usize },
    #[error("a cached value could not be decoded")]
    Decode(#[from] DecodeError),
    #[error("a SET value could not be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("the standalone cache fallback failed: {0}")]
    Fallback(#[from] RedisCacheError),
    #[error("the standalone delete failed: {0}")]
    Delete(#[from] RedisAdminError),
    #[error("the standalone set failed: {0}")]
    SetFallback(#[from] RedisCacheValuesError),
    #[error("the standalone script run failed: {0}")]
    Script(#[from] RedisCountersError),
}

#[derive(Debug, Error)]
pub enum PipelineFailure {
    #[error(transparent)]
    Breaker(#[from] BreakerOpen),
    #[error(transparent)]
    Client(#[from] ClientReuseError),
    #[error(transparent)]
    Live(#[from] LiveClientError),
    #[error("a SET value could not be serialized")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Ttl(#[from] RedisCountersError),
}
