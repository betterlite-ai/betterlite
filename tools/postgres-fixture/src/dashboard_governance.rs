use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::FixtureSeed;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DashboardGovernanceSeed;

#[derive(Debug, Error)]
pub enum DashboardGovernanceError {
    #[error("the dashboard_governance fixture seed is not built yet")]
    NotBuilt,
}

pub const fn validate(
    _governance: &DashboardGovernanceSeed,
    _seed: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    // failing here stops a governance case before any database or gateway starts
    Err(DashboardGovernanceError::NotBuilt)
}

#[expect(
    clippy::unused_async,
    reason = "the row inserts that replace this stub await the client"
)]
pub async fn seed(
    _client: &Client,
    governance: &DashboardGovernanceSeed,
    seed: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    validate(governance, seed)?;
    Ok(())
}
