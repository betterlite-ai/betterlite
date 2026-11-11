use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::FixtureSeed;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FinOpsSeed;

#[derive(Debug, Error)]
pub enum FinOpsFixtureError {
    #[error("the finops fixture seed is not built yet")]
    NotBuilt,
}

impl FinOpsSeed {
    pub const fn validate(&self, _fixture: &FixtureSeed) -> Result<(), FinOpsFixtureError> {
        // failing here stops a case that sets finops before any database or gateway starts
        Err(FinOpsFixtureError::NotBuilt)
    }
}

#[expect(
    clippy::unused_async,
    reason = "the row inserts that replace this stub await the client"
)]
pub async fn seed(_client: &Client, _finops: &FinOpsSeed) -> Result<(), FinOpsFixtureError> {
    // validation already rejects every finops seed, so only a direct caller reaches this
    Err(FinOpsFixtureError::NotBuilt)
}
