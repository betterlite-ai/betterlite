use std::error::Error as StdError;
use std::num::TryFromIntError;

use betterlite_db::postgres_store::errors::DbError;
use betterlite_roi::estimator::json::JsonError;
use thiserror::Error;

use crate::roi_cache::error::{CacheWriteError, ReportBlocked};
use crate::roi_github::error::SourceError;

use super::report::ReportError;

pub(crate) const REPLACED_MESSAGE: &str =
    "This sync was cancelled or replaced. Run analysis again to resume saved estimates.";

pub(crate) const GENERIC_MESSAGE: &str = "Unexpected source response. No partial report was saved. Check service compatibility and try again.";

#[derive(Debug, Error)]
pub(crate) enum RunError<Store>
where
    Store: StdError + 'static,
{
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Blocked(#[from] ReportBlocked),
    #[error(transparent)]
    Report(#[from] ReportError),
    #[error("the stored ROI data could not be read or written as JSON")]
    Json(#[from] serde_json::Error),
    #[error("the pull cache key could not be encoded")]
    Key(#[from] JsonError),
    #[error(transparent)]
    Cache(#[from] CacheWriteError<Store>),
    #[error("the gateway spend could not be read")]
    Spend(#[source] DbError),
    #[error("the stored report or pull record could not be read")]
    Repository(#[source] Store),
    #[error("the analysis date range is out of bounds")]
    DateRange,
    #[error("the analysis has more pull requests than it can count")]
    Count(#[source] TryFromIntError),
    #[error("{}", REPLACED_MESSAGE)]
    Replaced,
    #[error("The sync was cancelled")]
    Cancelled,
}

impl<Store> RunError<Store>
where
    Store: StdError + 'static,
{
    pub(crate) fn phase_error_message(&self) -> String {
        match self {
            Self::Source(error) => error.to_string(),
            Self::Blocked(error) => error.to_string(),
            Self::Replaced | Self::Cancelled => self.to_string(),
            Self::Report(_)
            | Self::Json(_)
            | Self::Key(_)
            | Self::Cache(_)
            | Self::Spend(_)
            | Self::Repository(_)
            | Self::DateRange
            | Self::Count(_) => GENERIC_MESSAGE.to_owned(),
        }
    }
}
