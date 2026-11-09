use redis::ParsingError;
use thiserror::Error;

use crate::cache_primitives::decode::DecodeError;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, Error)]
pub enum RedisCacheValuesError {
    #[error(transparent)]
    BreakerOpen(#[from] BreakerOpen),
    #[error(transparent)]
    ClientReuse(#[from] ClientReuseError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Live(#[from] LiveClientError),
    #[error(transparent)]
    Reply(#[from] ParsingError),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
    #[error("the TTL of {seconds} seconds does not fit a Redis expiry")]
    TtlOutOfRange { seconds: f64 },
}
