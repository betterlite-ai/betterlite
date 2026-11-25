use std::error::Error as StdError;

use betterlite_auth::reservation_settlement::error::SettlementError;
use betterlite_auth::spend_increments::error::SpendIncrementError;
use thiserror::Error;

use crate::spend_cache_update::SpendCacheUpdateError;

#[derive(Debug, Error)]
pub enum SpendSettlementError<WriteError>
where
    WriteError: StdError + Send + Sync + 'static,
{
    #[error("the spend write failed")]
    Write(#[source] WriteError),
    #[error("the budget reservation of an uncharged request could not be released")]
    Release(#[source] SettlementError),
    #[error("the update_cache read could not be armed on the request pipeline")]
    Prefetch(#[source] SpendCacheUpdateError),
    #[error("the spend counters could not be incremented after the spend was persisted")]
    Counters(#[source] SpendIncrementError),
}
