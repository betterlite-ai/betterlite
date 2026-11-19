use std::error::Error;
use std::sync::Arc;

use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};
use futures_util::future::BoxFuture;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

use super::error::{FocusError, FocusInitError, ScheduleError};
use super::export::{FocusEngineFactory, FocusExporter, FocusLogger};

#[path = "export_schedule/tests/exact_type_match.rs"]
mod exact_type_match;
#[path = "export_schedule/tests/pod_lock_acquire.rs"]
mod pod_lock_acquire;
#[path = "export_schedule/tests/pod_lock_acquire_fail.rs"]
mod pod_lock_acquire_fail;
#[path = "export_schedule/tests/pod_lock_manager_adapter.rs"]
mod pod_lock_manager_adapter;
#[path = "export_schedule/tests/pod_lock_release.rs"]
mod pod_lock_release;
#[path = "export_schedule/tests/pod_lock_unavailable.rs"]
mod pod_lock_unavailable;
#[path = "export_schedule/tests/scheduler_registration.rs"]
mod scheduler_registration;
#[path = "export_schedule/tests/scheduler_skip_empty.rs"]
mod scheduler_skip_empty;
#[path = "export_schedule/tests/trigger_daily.rs"]
mod trigger_daily;
#[path = "export_schedule/tests/trigger_hourly.rs"]
mod trigger_hourly;
#[path = "export_schedule/tests/trigger_interval.rs"]
mod trigger_interval;
#[path = "export_schedule/tests/trigger_unsupported.rs"]
mod trigger_unsupported;
#[path = "export_schedule/tests/window_daily.rs"]
mod window_daily;
#[path = "export_schedule/tests/window_hourly.rs"]
mod window_hourly;
#[path = "export_schedule/tests/window_interval.rs"]
mod window_interval;
#[path = "export_schedule/tests/window_unsupported.rs"]
mod window_unsupported;

pub const FOCUS_USAGE_DATA_JOB_NAME: &str = "focus_export_usage_data";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleTrigger {
    Interval {
        seconds: i64,
    },
    Cron {
        hour: Option<i64>,
        minute: i64,
        second: i64,
    },
}

// the frequencies the trigger and the window know: any other configured text rides along so the error can name it
pub enum ExportFrequency {
    Hourly,
    Daily,
    Interval,
    Unsupported(String),
}

impl ExportFrequency {
    fn parse(frequency: &str) -> Self {
        match frequency {
            "hourly" => Self::Hourly,
            "daily" => Self::Daily,
            "interval" => Self::Interval,
            other => Self::Unsupported(other.to_owned()),
        }
    }
}

// type(cb) is FocusLogger on a registered callback: VantageLogger and any other subclass carry their own tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusLoggerType {
    Focus,
    Vantage,
}

pub struct RegisteredFocusLogger<F: FocusEngineFactory> {
    pub logger: Arc<FocusLogger<F>>,
    pub logger_type: FocusLoggerType,
}

// APScheduler's add_job, reduced to the call init makes: a job and its trigger kwargs.
pub trait FocusScheduler<F: FocusEngineFactory> {
    type Error: Error + Send + Sync + 'static;

    fn add_job(
        &self,
        logger: Arc<FocusLogger<F>>,
        trigger: ScheduleTrigger,
    ) -> Result<(), Self::Error>;
}

// The redis-backed pod lock of PodLockManager, behind a port so a test needs no redis.
pub trait FocusPodLock: Send + Sync {
    fn has_cache(&self) -> bool;

    fn acquire_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, Option<bool>>;

    fn release_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, ()>;
}

impl<Cache: LockCache> FocusPodLock for PodLockManager<Cache> {
    fn has_cache(&self) -> bool {
        self.cache.is_some()
    }

    fn acquire_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, Option<bool>> {
        Box::pin(Self::acquire_lock(self, cronjob_id, None, true))
    }

    fn release_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, ()> {
        Box::pin(Self::release_lock(self, cronjob_id))
    }
}

// The redis lease of one scheduled run: a tracked task releases the job's lock whether the export ends or its future is dropped.
pub(crate) struct PodLockLease {
    release_now: Arc<Notify>,
    release_task: Option<JoinHandle<()>>,
}

impl PodLockLease {
    pub(crate) fn start(
        pod_lock: Arc<dyn FocusPodLock>,
        job_name: &'static str,
        tasks: &TaskTracker,
    ) -> Self {
        let release_now = Arc::new(Notify::new());
        let wake = Arc::clone(&release_now);
        let release_task = tasks.spawn(async move {
            wake.notified().await;
            pod_lock.release_lock(job_name).await;
        });
        Self {
            release_now,
            release_task: Some(release_task),
        }
    }

    pub(crate) async fn release(mut self) {
        self.release_now.notify_one();
        if let Some(release_task) = self.release_task.take() {
            let _released = release_task.await;
        }
    }
}

impl Drop for PodLockLease {
    fn drop(&mut self) {
        // a cancelled export never reaches release(): wake the tracked task so the lease is not held until its TTL
        self.release_now.notify_one();
    }
}

// _build_scheduler_trigger's daily branch: the offset counts minutes from midnight, clamped into its day
pub(crate) fn daily_cron_trigger(cron_offset_minute: i64) -> ScheduleTrigger {
    let total_minutes = cron_offset_minute.max(0);
    ScheduleTrigger::Cron {
        hour: Some(total_minutes.div_euclid(60).min(23)),
        minute: total_minutes.rem_euclid(60).min(59),
        second: 0,
    }
}

// The scheduling half of the logger: free functions here, so FocusLogger keeps its one inherent impl in export.rs.
pub fn build_scheduler_trigger<F>(logger: &FocusLogger<F>) -> Result<ScheduleTrigger, ScheduleError>
where
    F: FocusEngineFactory,
{
    match ExportFrequency::parse(&logger.frequency) {
        ExportFrequency::Interval => Ok(ScheduleTrigger::Interval {
            seconds: logger.interval_seconds,
        }),
        ExportFrequency::Hourly => Ok(ScheduleTrigger::Cron {
            hour: None,
            minute: logger.cron_offset_minute.clamp(0, 59),
            second: 0,
        }),
        ExportFrequency::Daily => Ok(daily_cron_trigger(logger.cron_offset_minute)),
        ExportFrequency::Unsupported(frequency) => {
            Err(ScheduleError::UnsupportedFrequency { frequency })
        }
    }
}

pub fn compute_time_window<F>(
    logger: &FocusLogger<F>,
    now: DateTime<Utc>,
) -> Result<FocusTimeWindow, ScheduleError>
where
    F: FocusEngineFactory,
{
    let (start_time, end_time) = match ExportFrequency::parse(&logger.frequency) {
        ExportFrequency::Hourly => truncated_window(now, TimeDelta::try_hours(1))?,
        ExportFrequency::Daily => truncated_window(now, TimeDelta::try_days(1))?,
        ExportFrequency::Interval => {
            let interval = TimeDelta::try_seconds(logger.interval_seconds)
                .ok_or(ScheduleError::TimeWindowOutOfRange)?;
            let start_time = now
                .checked_sub_signed(interval)
                .ok_or(ScheduleError::TimeWindowOutOfRange)?;
            (start_time, now)
        }
        ExportFrequency::Unsupported(frequency) => {
            return Err(ScheduleError::UnsupportedFrequency { frequency });
        }
    };
    Ok(FocusTimeWindow {
        start_time,
        end_time,
        frequency: logger.frequency.clone(),
    })
}

pub async fn initialize_focus_export_job<F>(
    logger: &FocusLogger<F>,
) -> Result<(), FocusError<<F::Engine as FocusExporter>::Error>>
where
    F: FocusEngineFactory,
{
    let lease = match logger.pod_lock.as_ref() {
        Some(pod_lock) if pod_lock.has_cache() => {
            if pod_lock.acquire_lock(FOCUS_USAGE_DATA_JOB_NAME).await != Some(true) {
                tracing::debug!("Focus export: unable to acquire pod lock");
                return Ok(());
            }
            Some(PodLockLease::start(
                Arc::clone(pod_lock),
                FOCUS_USAGE_DATA_JOB_NAME,
                &logger.tasks,
            ))
        }
        _ => None,
    };
    let computed = compute_time_window(logger, logger.clock.now());
    let result = match computed {
        Ok(window) => {
            logger
                .export_usage_data(None, Some(window.start_time), Some(window.end_time))
                .await
        }
        Err(error) => Err(FocusError::Schedule(error)),
    };
    if let Some(held) = lease {
        held.release().await;
    }
    result
}

pub fn init_focus_export_job<F, S>(
    loggers: &[RegisteredFocusLogger<F>],
    scheduler: &S,
) -> Result<(), FocusInitError<S::Error>>
where
    F: FocusEngineFactory,
    S: FocusScheduler<F>,
{
    let Some(entry) = loggers
        .iter()
        .find(|entry| entry.logger_type == FocusLoggerType::Focus)
    else {
        tracing::debug!("No Focus export logger registered; skipping scheduler");
        return Ok(());
    };
    let trigger = build_scheduler_trigger(&entry.logger)?;
    scheduler
        .add_job(Arc::clone(&entry.logger), trigger)
        .map_err(FocusInitError::Scheduler)
}

// now truncated to the start of its hour or day, with the previous whole span as the start
fn truncated_window(
    now: DateTime<Utc>,
    span: Option<TimeDelta>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), ScheduleError> {
    let duration = span.ok_or(ScheduleError::TimeWindowOutOfRange)?;
    let end_time = now
        .duration_trunc(duration)
        .map_err(|_error| ScheduleError::TimeWindowOutOfRange)?;
    let start_time = end_time
        .checked_sub_signed(duration)
        .ok_or(ScheduleError::TimeWindowOutOfRange)?;
    Ok((start_time, end_time))
}
