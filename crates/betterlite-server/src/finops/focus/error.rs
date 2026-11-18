use std::error::Error;

use betterlite_env::EnvError;

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("Unsupported frequency: {frequency}")]
    UnsupportedFrequency { frequency: String },
    #[error("the FOCUS export time window is out of range")]
    TimeWindowOutOfRange,
}

#[derive(Debug, thiserror::Error)]
pub enum FocusError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("start_time_utc and end_time_utc must be provided together")]
    UnpairedBounds,
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error("the FOCUS export failed")]
    Export(#[source] E),
}

#[derive(Debug, thiserror::Error)]
pub enum FocusInitError<SchedulerError>
where
    SchedulerError: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error("the FOCUS export job could not be scheduled")]
    Scheduler(#[source] SchedulerError),
}
