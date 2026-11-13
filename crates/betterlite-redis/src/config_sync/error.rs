use rand::rngs::SysError;

use crate::redis_client_reuse::error::ClientReuseError;

#[derive(Debug, thiserror::Error)]
pub enum ConfigSyncError {
    #[error(transparent)]
    Client(#[from] ClientReuseError),
    #[error("the config change message could not be serialized")]
    Message(#[from] serde_json::Error),
    #[error("the config sync jitter source could not be seeded")]
    Entropy(#[from] SysError),
}
