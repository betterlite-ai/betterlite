use tokio::task::JoinError;

use crate::cache_primitives::decode::DecodeError;
use crate::redis_client_factories::error::FactoryError;
use crate::redis_client_reuse::breaker::BreakerOpen;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_connection_settings::error::RedisSettingsError;
use crate::redis_counters::RedisCountersError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, thiserror::Error)]
pub enum RedisCacheError {
    #[error(transparent)]
    Breaker(#[from] BreakerOpen),
    #[error(transparent)]
    ClientReuse(#[from] ClientReuseError),
    #[error(transparent)]
    LiveClient(#[from] LiveClientError),
    #[error(transparent)]
    Counters(#[from] RedisCountersError),
    #[error(transparent)]
    Settings(#[from] RedisSettingsError),
    #[error(transparent)]
    Factory(#[from] FactoryError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Join(#[from] JoinError),
}

// litellm's init_async_client builds a lazy client, so a failed connection or an unreadable TLS file lands in the method body; only building the client record sits outside it
pub(crate) const fn connection_failure(error: &ClientReuseError) -> bool {
    matches!(
        error,
        ClientReuseError::Live(
            LiveClientError::Redis(_)
                | LiveClientError::NoSentinelMaster { .. }
                | LiveClientError::TlsFile { .. }
        ) | ClientReuseError::Timeout(_)
    )
}
