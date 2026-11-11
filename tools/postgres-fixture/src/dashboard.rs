use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::FixtureSeed;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DashboardSeed;

#[derive(Debug, Error)]
pub enum DashboardFixtureError {
    #[error("the dashboard fixture seed is not built yet")]
    NotBuilt,
}

pub const fn validate(
    _dashboard: &DashboardSeed,
    _seed: &FixtureSeed,
) -> Result<(), DashboardFixtureError> {
    // failing here stops a case that sets dashboard before any database or gateway starts
    Err(DashboardFixtureError::NotBuilt)
}

#[expect(
    clippy::unused_async,
    reason = "the schema statements that replace this stub await the client"
)]
pub async fn initialize(_client: &Client) -> Result<(), DashboardFixtureError> {
    // validation already rejects every dashboard seed, so only a direct caller reaches this
    Err(DashboardFixtureError::NotBuilt)
}

#[expect(
    clippy::unused_async,
    reason = "the log inserts that replace this stub await the client"
)]
pub async fn seed(
    _client: &Client,
    dashboard: &DashboardSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardFixtureError> {
    validate(dashboard, seed)?;
    Ok(())
}
