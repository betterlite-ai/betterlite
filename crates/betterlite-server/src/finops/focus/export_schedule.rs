use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::focus_export_schedule::DEFAULT_INTERVAL_SECONDS;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};
use futures_util::future::BoxFuture;

use super::error::{FocusError, FocusInitError, ScheduleError};
use super::export::{FocusEngineFactory, FocusLogger};

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
        Box::pin(PodLockManager::acquire_lock(self, cronjob_id, None, true))
    }

    fn release_lock<'job>(&'job self, cronjob_id: &'job str) -> BoxFuture<'job, ()> {
        Box::pin(PodLockManager::release_lock(self, cronjob_id))
    }
}

impl<F: FocusEngineFactory> FocusLogger<F> {
    pub fn build_scheduler_trigger(&self) -> Result<ScheduleTrigger, ScheduleError> {
        match self.frequency.as_str() {
            "interval" => Ok(ScheduleTrigger::Interval {
                seconds: self.interval_seconds.unwrap_or(DEFAULT_INTERVAL_SECONDS),
            }),
            "hourly" => Ok(ScheduleTrigger::Cron {
                hour: None,
                minute: self.cron_offset_minute.clamp(0, 59),
                second: 0,
            }),
            "daily" => {
                let total_minutes = self.cron_offset_minute.max(0);
                Ok(ScheduleTrigger::Cron {
                    hour: Some(total_minutes.div_euclid(60).min(23)),
                    minute: total_minutes.rem_euclid(60).min(59),
                    second: 0,
                })
            }
            _ => Err(ScheduleError::UnsupportedFrequency {
                frequency: self.frequency.clone(),
            }),
        }
    }

    pub fn compute_time_window(
        &self,
        now: DateTime<Utc>,
    ) -> Result<FocusTimeWindow, ScheduleError> {
        let (start_time, end_time) = match self.frequency.as_str() {
            "hourly" => truncated_window(now, TimeDelta::try_hours(1))?,
            "daily" => truncated_window(now, TimeDelta::try_days(1))?,
            "interval" => {
                let seconds = self.interval_seconds.unwrap_or(DEFAULT_INTERVAL_SECONDS);
                let interval =
                    TimeDelta::try_seconds(seconds).ok_or(ScheduleError::TimeWindowOutOfRange)?;
                let start_time = now
                    .checked_sub_signed(interval)
                    .ok_or(ScheduleError::TimeWindowOutOfRange)?;
                (start_time, now)
            }
            _ => {
                return Err(ScheduleError::UnsupportedFrequency {
                    frequency: self.frequency.clone(),
                });
            }
        };
        Ok(FocusTimeWindow {
            start_time,
            end_time,
            frequency: self.frequency.clone(),
        })
    }

    pub async fn initialize_focus_export_job(&self) -> Result<(), FocusError<F::Engine::Error>> {
        let locked = match self.pod_lock.as_ref() {
            Some(pod_lock) if pod_lock.has_cache() => {
                if pod_lock.acquire_lock(FOCUS_USAGE_DATA_JOB_NAME).await != Some(true) {
                    tracing::debug!("Focus export: unable to acquire pod lock");
                    return Ok(());
                }
                Some(pod_lock)
            }
            _ => None,
        };
        let computed = self.compute_time_window(self.clock.now());
        let result = match computed {
            Ok(window) => {
                self.export_usage_data(None, Some(window.start_time), Some(window.end_time))
                    .await
            }
            Err(error) => Err(FocusError::Schedule(error)),
        };
        if let Some(pod_lock) = locked {
            pod_lock.release_lock(FOCUS_USAGE_DATA_JOB_NAME).await;
        }
        result
    }
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
    let trigger = entry.logger.build_scheduler_trigger()?;
    scheduler
        .add_job(Arc::clone(&entry.logger), trigger)
        .map_err(FocusInitError::Scheduler)
}

// now truncated to the start of its hour or day, with the previous whole span as the start
fn truncated_window(
    now: DateTime<Utc>,
    span: Option<TimeDelta>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), ScheduleError> {
    let span = span.ok_or(ScheduleError::TimeWindowOutOfRange)?;
    let end_time = now
        .duration_trunc(span)
        .map_err(|_error| ScheduleError::TimeWindowOutOfRange)?;
    let start_time = end_time
        .checked_sub_signed(span)
        .ok_or(ScheduleError::TimeWindowOutOfRange)?;
    Ok((start_time, end_time))
}
