use betterlite_env::EnvError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;
use tokio::task::JoinError;

use crate::proxy_request_contracts::ContractError;
use crate::redis_cache_wiring::build::RedisCacheWiringError;

#[derive(Debug, Error)]
pub enum CoordRedisError {
    #[error("general_settings.coordination_redis must be a mapping of Redis connection params")]
    NotMapping,
    #[error(
        "general_settings.coordination_redis needs a connection target: set one of host, url, startup_nodes, or sentinel_nodes"
    )]
    MissingTarget,
    #[error(transparent)]
    Params(#[from] ContractError),
    #[error("the coordination Redis settings could not be read: {source}")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
    #[error("REDIS_CLUSTER_NODES is not valid JSON: {source}")]
    ClusterNodes {
        #[source]
        source: serde_json::Error,
    },
    #[error("the coordination Redis params could not be read as Redis settings: {source}")]
    Overrides {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Build(#[from] RedisCacheWiringError),
    #[error(transparent)]
    Secrets(#[from] SecretLookupError),
    #[error("the coordination Redis secret lookup task failed: {source}")]
    LookupTask {
        #[from]
        source: JoinError,
    },
    #[error("the coordination Redis secret lookup task ended without a result")]
    LookupMissing,
    #[error(transparent)]
    Environment(#[from] EnvError),
}
