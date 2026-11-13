use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySeed {
    Empty,
    Catalog,
    Limit,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("the discovery_skills fixture seed is not built yet")]
    NotBuilt,
}

#[expect(
    clippy::unused_async,
    reason = "the table and row inserts that replace this stub await the client"
)]
pub async fn seed(_client: &Client, _seed: &DiscoverySeed) -> Result<(), DiscoveryError> {
    // failing here makes fixture setup clean up instead of starting gateways on a missing table
    Err(DiscoveryError::NotBuilt)
}
