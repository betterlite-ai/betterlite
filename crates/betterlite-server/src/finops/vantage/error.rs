use std::error::Error;

use betterlite_env::EnvError;

use crate::finops::focus::error::{FocusError, ScheduleError};

#[derive(Debug, thiserror::Error)]
pub enum VantageExportError {
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("start_time_utc and end_time_utc must be provided together")]
    UnpairedBounds,
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error("the VANTAGE export failed")]
    Export(#[source] Box<dyn Error + Send + Sync>),
}

impl<E> From<FocusError<E>> for VantageExportError
where
    E: Error + Send + Sync + 'static,
{
    fn from(error: FocusError<E>) -> Self {
        match error {
            FocusError::Environment(source) => Self::Environment(source),
            FocusError::UnpairedBounds => Self::UnpairedBounds,
            FocusError::Schedule(source) => Self::Schedule(source),
            FocusError::Export(source) => Self::Export(Box::new(source)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VantageInitError<SchedulerError>
where
    SchedulerError: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error("the VANTAGE export job could not be scheduled")]
    Scheduler(#[source] SchedulerError),
}
