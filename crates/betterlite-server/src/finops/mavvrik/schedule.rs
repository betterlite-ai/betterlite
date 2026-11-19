use std::error::Error;
use std::sync::Arc;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

use super::error::{InitError, MavvrikError};
use super::export::{
    MAVVRIK_FOCUS_EXPORT_JOB_NAME, MavvrikEngineFactory, MavvrikExporter, MavvrikFocusLogger,
};
use super::scheduled_export::run_scheduled_export;
use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleTrigger};

#[path = "schedule/tests/exact_type_match.rs"]
mod exact_type_match;
#[path = "schedule/tests/lazy_init.rs"]
mod lazy_init;
#[path = "schedule/tests/pod_lock_acquire.rs"]
mod pod_lock_acquire;
#[path = "schedule/tests/pod_lock_acquire_fail.rs"]
mod pod_lock_acquire_fail;
#[path = "schedule/tests/pod_lock_release.rs"]
mod pod_lock_release;
#[path = "schedule/tests/registration.rs"]
mod registration;
#[path = "schedule/tests/skip_empty.rs"]
mod skip_empty;

// _init_custom_logger_compatible_class's logging_integration: the callback name litellm.callbacks holds
const MAVVRIK_INTEGRATION: &str = "mavvrik";

// The redis lease of one scheduled run: a tracked task releases it whether the export ends or its future is dropped.
struct MavvrikLease {
    release_now: Arc<Notify>,
    released: Option<JoinHandle<()>>,
}

impl MavvrikLease {
    fn start(pod_lock: Arc<dyn FocusPodLock>, tasks: &TaskTracker) -> Self {
        let release_now = Arc::new(Notify::new());
        let released = tasks.spawn(release_on_signal(Arc::clone(&release_now), pod_lock));
        Self {
            release_now,
            released: Some(released),
        }
    }

    async fn release(mut self) {
        self.release_now.notify_one();
        if let Some(released) = self.released.take() {
            let _released = released.await;
        }
    }
}

impl Drop for MavvrikLease {
    fn drop(&mut self) {
        // the export's future may be dropped before release(): the tracked task still frees the lock
        self.release_now.notify_one();
    }
}

async fn release_on_signal(release_now: Arc<Notify>, pod_lock: Arc<dyn FocusPodLock>) {
    release_now.notified().await;
    pod_lock.release_lock(MAVVRIK_FOCUS_EXPORT_JOB_NAME).await;
}

// initialize_mavvrik_focus_export_job: no lock manager and no redis mean no lease to take, and the export runs alone
pub async fn initialize_mavvrik_export_job<F>(
    logger: &MavvrikFocusLogger<F>,
) -> Result<(), MavvrikError<<F::Engine as MavvrikExporter>::Error>>
where
    F: MavvrikEngineFactory,
{
    let lease = match logger.pod_lock.as_ref() {
        Some(pod_lock) if pod_lock.has_cache() => {
            if pod_lock.acquire_lock(MAVVRIK_FOCUS_EXPORT_JOB_NAME).await != Some(true) {
                tracing::debug!("Mavvrik FOCUS export: unable to acquire pod lock");
                return Ok(());
            }
            Some(MavvrikLease::start(Arc::clone(pod_lock), &logger.tasks))
        }
        _ => None,
    };
    let result = run_scheduled_export(logger).await;
    if let Some(held) = lease {
        held.release().await;
    }
    result
}

// type(cb) is MavvrikFocusLogger on a registered callback: a subclass carries the other tag
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MavvrikLoggerType {
    Mavvrik,
    Subclass,
}

pub struct RegisteredMavvrikLogger<F: MavvrikEngineFactory> {
    pub logger: Arc<MavvrikFocusLogger<F>>,
    pub logger_type: MavvrikLoggerType,
}

// APScheduler's add_job, reduced to the arguments this job passes: its method, id, trigger and replace_existing
pub trait MavvrikScheduler<F: MavvrikEngineFactory> {
    type Error: Error + Send + Sync + 'static;

    fn add_job(
        &self,
        logger: Arc<MavvrikFocusLogger<F>>,
        job_id: &str,
        trigger: ScheduleTrigger,
        replace_existing: bool,
    ) -> Result<(), Self::Error>;
}

// _init_custom_logger_compatible_class(logging_integration="mavvrik", ...): the registry builds the configured callback
pub trait MavvrikLazyInit<F: MavvrikEngineFactory> {
    fn initialize(&self, logging_integration: &str) -> Option<Arc<MavvrikFocusLogger<F>>>;
}

// _build_scheduler_trigger's daily branch: the offset counts minutes from midnight, clamped into its day
fn daily_trigger<F>(logger: &MavvrikFocusLogger<F>) -> ScheduleTrigger
where
    F: MavvrikEngineFactory,
{
    let total_minutes = logger.cron_offset_minute.max(0);
    ScheduleTrigger::Cron {
        hour: Some(total_minutes.div_euclid(60).min(23)),
        minute: total_minutes.rem_euclid(60).min(59),
        second: 0,
    }
}

// init_mavvrik_focus_background_job: only an exact-type logger is scheduled, and a configured name builds one lazily
fn select_logger<F, L>(
    registered: &[RegisteredMavvrikLogger<F>],
    configured: bool,
    lazy_init: &L,
) -> Option<Arc<MavvrikFocusLogger<F>>>
where
    F: MavvrikEngineFactory,
    L: MavvrikLazyInit<F>,
{
    let exact = registered
        .iter()
        .find(|entry| entry.logger_type == MavvrikLoggerType::Mavvrik);
    if let Some(entry) = exact {
        return Some(Arc::clone(&entry.logger));
    }
    if configured {
        return lazy_init.initialize(MAVVRIK_INTEGRATION);
    }
    None
}

pub fn init_mavvrik_background_job<F, L, S>(
    registered: &[RegisteredMavvrikLogger<F>],
    configured: bool,
    lazy_init: &L,
    scheduler: &S,
) -> Result<(), InitError<S::Error>>
where
    F: MavvrikEngineFactory,
    L: MavvrikLazyInit<F>,
    S: MavvrikScheduler<F>,
{
    let Some(logger) = select_logger(registered, configured, lazy_init) else {
        tracing::debug!("No MavvrikFocusLogger registered; skipping scheduler");
        return Ok(());
    };
    let trigger = daily_trigger(&logger);
    scheduler
        .add_job(
            Arc::clone(&logger),
            MAVVRIK_FOCUS_EXPORT_JOB_NAME,
            trigger,
            true,
        )
        .map_err(InitError::Scheduler)?;
    tracing::info!("mavvrik_focus: background export job scheduled ({trigger:?})");
    Ok(())
}
