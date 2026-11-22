use betterlite_config::sections::settings::ConfigOwnedKeyError;

use crate::job_scheduler::SchedulerError;

#[derive(Debug, thiserror::Error)]
pub enum CleanupScheduleError {
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error(transparent)]
    ConfigOwned(#[from] ConfigOwnedKeyError),
}
