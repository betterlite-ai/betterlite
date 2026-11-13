use tokio::time::error::Elapsed;

use crate::redis_client_factories::error::FactoryError;
use crate::redis_live_client::LiveClientError;

#[derive(Debug, thiserror::Error)]
pub enum ClientReuseError {
    #[error(transparent)]
    Live(#[from] LiveClientError),
    #[error(transparent)]
    Factory(#[from] FactoryError),
    #[error(
        "cannot derive a pub/sub client: redis cluster has no default node and no startup nodes"
    )]
    NoPubSubNode,
    #[error("redis cluster connections do not carry a pub/sub client")]
    ClusterPubSub,
    #[error("the redis connection could not be established in time")]
    Timeout(#[from] Elapsed),
}
