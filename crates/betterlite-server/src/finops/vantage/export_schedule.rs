use std::error::Error;
use std::sync::Arc;

use tokio_util::task::TaskTracker;

use crate::finops::focus::export::FocusEngineFactory;
use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleTrigger, compute_time_window};

use super::VANTAGE_USAGE_DATA_JOB_NAME;
use super::error::{VantageExportError, VantageInitError};
use super::export::{VantageExportLogger, VantageLogger};
use super::store::VantageLoggers;

#[path = "export_schedule/tests/pod_lock_fail.rs"]
mod pod_lock_fail;
#[path = "export_schedule/tests/pod_lock_key.rs"]
mod pod_lock_key;
#[path = "export_schedule/tests/pod_lock_release.rs"]
mod pod_lock_release;
#[path = "export_schedule/tests/pod_lock_unavailable.rs"]
mod pod_lock_unavailable;
#[path = "export_schedule/tests/scheduler_registration.rs"]
mod scheduler_registration;
#[path = "export_schedule/tests/scheduler_skip_empty.rs"]
mod scheduler_skip_empty;
#[path = "export_schedule/tests/scheduler_type_match.rs"]
mod scheduler_type_match;

// APScheduler's add_job, reduced to the call init makes: a job and its trigger.
pub trait VantageScheduler {
    type Error: Error + Send + Sync + 'static;

    fn add_job(
        &self,
        logger: Arc<dyn VantageExportLogger>,
        trigger: ScheduleTrigger,
    ) -> Result<(), Self::Error>;
}

// The redis lease of one scheduled run, released under Vantage's own key so it never contends with FocusLogger.
struct PodLockRelease {
    pod_lock: Option<Arc<dyn FocusPodLock>>,
    tasks: TaskTracker,
}

impl PodLockRelease {
    async fn release(mut self) {
        let Some(pod_lock) = self.pod_lock.as_ref().map(Arc::clone) else {
            return;
        };
        pod_lock.release_lock(VANTAGE_USAGE_DATA_JOB_NAME).await;
        self.pod_lock = None;
    }
}

impl Drop for PodLockRelease {
    fn drop(&mut self) {
        // a cancelled run never reaches release(): the tracked task finishes the release after the drop
        let Some(pod_lock) = self.pod_lock.take() else {
            return;
        };
        self.tasks.spawn(async move {
            pod_lock.release_lock(VANTAGE_USAGE_DATA_JOB_NAME).await;
        });
    }
}

// VantageLogger.initialize_focus_export_job: FocusLogger's locking under the Vantage lease key.
pub async fn initialize_vantage_export_job<F>(
    logger: &VantageLogger<F>,
) -> Result<(), VantageExportError>
where
    F: FocusEngineFactory,
{
    let lease = match logger.pod_lock.as_ref() {
        Some(pod_lock) if pod_lock.has_cache() => {
            if pod_lock.acquire_lock(VANTAGE_USAGE_DATA_JOB_NAME).await != Some(true) {
                tracing::debug!("Vantage export: unable to acquire pod lock");
                return Ok(());
            }
            Some(PodLockRelease {
                pod_lock: Some(Arc::clone(pod_lock)),
                tasks: logger.tasks.clone(),
            })
        }
        _ => None,
    };
    let result = scheduled_export(logger).await;
    if let Some(held) = lease {
        held.release().await;
    }
    result
}

async fn scheduled_export<F>(logger: &VantageLogger<F>) -> Result<(), VantageExportError>
where
    F: FocusEngineFactory,
{
    let window = compute_time_window(&logger.focus, logger.clock.now())
        .map_err(VantageExportError::Schedule)?;
    logger
        .export_usage_data(None, Some(window.start_time), Some(window.end_time))
        .await
}

// VantageLogger.init_vantage_background_job: the first registered instance on the interval or cron trigger.
pub fn init_vantage_background_job<S>(
    loggers: &VantageLoggers,
    scheduler: &S,
) -> Result<(), VantageInitError<S::Error>>
where
    S: VantageScheduler,
{
    let Some(logger) = loggers.snapshot().into_iter().next() else {
        tracing::debug!("No Vantage logger registered; skipping scheduler");
        return Ok(());
    };
    let trigger = logger.scheduler_trigger()?;
    scheduler
        .add_job(logger, trigger)
        .map_err(VantageInitError::Scheduler)
}
