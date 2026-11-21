use std::error::Error;

use crate::database_faults::DbError;

use super::json_persist::WindowJsonError;
use super::window_expiry::WindowExpiryError;

// One source's walk ends on any of these; reset_budget_windows catches it, logs the source and moves on to the next source.
#[derive(Debug, thiserror::Error)]
pub enum WindowResetError<ScheduleFailure>
where
    ScheduleFailure: Error + 'static,
{
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Json(#[from] WindowJsonError),
    #[error("the reset_at of a window on row {row} is malformed: {source}")]
    Expiry {
        row: String,
        #[source]
        source: WindowExpiryError,
    },
    #[error("the budget reset schedule refused the next reset time: {source}")]
    Schedule {
        #[source]
        source: ScheduleFailure,
    },
}
