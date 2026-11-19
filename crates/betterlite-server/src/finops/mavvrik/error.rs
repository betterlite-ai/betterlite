use std::error::Error;

use betterlite_env::EnvError;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "MAVVRIK_FOCUS_FREQUENCY='{frequency}' is not supported; only daily is allowed because the Mavvrik ingestion protocol stores one file per calendar date"
    )]
    UnsupportedFrequency { frequency: String },
    #[error(transparent)]
    Environment(#[from] EnvError),
}

#[derive(Debug, thiserror::Error)]
pub enum MavvrikError<EngineError>
where
    EngineError: Error + Send + Sync + 'static,
{
    #[error("the Mavvrik FOCUS export failed")]
    Export(#[source] EngineError),
    #[error("the Mavvrik FOCUS export time window is out of range")]
    TimeWindowOutOfRange,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError<SchedulerError>
where
    SchedulerError: Error + Send + Sync + 'static,
{
    #[error("the Mavvrik FOCUS export job could not be scheduled")]
    Scheduler(#[source] SchedulerError),
}
