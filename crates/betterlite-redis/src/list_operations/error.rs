use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, thiserror::Error)]
pub enum RedisListOperationsError {
    #[error(transparent)]
    BreakerOpen(#[from] BreakerOpen),
    #[error(transparent)]
    ClientReuse(#[from] ClientReuseError),
    #[error(transparent)]
    Live(#[from] LiveClientError),
    #[error("redis returned an unexpected reply for {operation}")]
    UnexpectedReply { operation: &'static str },
    #[error("the trim length {max_len} cannot be negated")]
    TrimLength { max_len: i64 },
}
