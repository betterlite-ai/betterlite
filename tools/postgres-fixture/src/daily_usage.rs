use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

use crate::seed::KeyRow;

pub const SCHEMA: &str = "";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DailyUsageRow;

#[derive(Debug, Error)]
pub enum DailyUsageError {
    #[error("the daily_usage fixture seed is not built yet")]
    NotBuilt,
}

pub const fn validate(_rows: &[DailyUsageRow], _keys: &[KeyRow]) -> Result<(), DailyUsageError> {
    // failing here stops a case that sets daily_usage before any database or gateway starts
    Err(DailyUsageError::NotBuilt)
}

#[expect(
    clippy::unused_async,
    reason = "the row inserts that replace this stub await the client"
)]
pub async fn seed(
    _client: &Client,
    rows: &[DailyUsageRow],
    keys: &[KeyRow],
) -> Result<(), DailyUsageError> {
    validate(rows, keys)?;
    Ok(())
}
