use std::sync::Arc;

use rand::Rng;
use rand::RngExt as _;

use betterlite_config::sections::settings::JsonValue;

use crate::job_scheduler::JobFn;
use crate::job_scheduler::triggers::{CronTrigger, CronZone, IntervalTrigger};

use super::changes::CleanupScheduleState;
use super::error::CleanupScheduleError;
use super::ports::{CleanupScheduler, GeneralSettingsView, cleanup_job_spec};
use super::schedule::{CleanupSchedule, cleanup_schedule_of, value_text, wants_cleanup_job};
use super::trigger::{
    ParsedInterval, ScheduleContext, interval_past_datetime_max, parsed_interval,
};

pub struct CleanupRegistration<'registration> {
    pub scheduler: &'registration mut dyn CleanupScheduler,
    pub settings: &'registration dyn GeneralSettingsView,
    pub context: ScheduleContext<'registration>,
    pub job: JobFn,
}

pub fn register_cleanup_job<R>(
    registration: &mut CleanupRegistration<'_>,
    rng: &mut R,
    state: &mut CleanupScheduleState,
) -> Result<(), CleanupScheduleError>
where
    R: Rng + ?Sized,
{
    let schedule = cleanup_schedule_of(registration.settings);
    if !wants_cleanup_job(&schedule) {
        return Ok(());
    }
    match schedule.cleanup_cron.as_ref() {
        Some(JsonValue::String(text)) => {
            startup_cron(registration.scheduler, text, Arc::clone(&registration.job))?;
        }
        Some(value) => startup_invalid_cron(value),
        None => {
            startup_interval(
                registration.scheduler,
                &schedule,
                &registration.context,
                Arc::clone(&registration.job),
                rng,
            )?;
        }
    }
    state.last_attempt = Some(schedule);
    Ok(())
}

fn startup_cron(
    scheduler: &mut dyn CleanupScheduler,
    text: &str,
    job: JobFn,
) -> Result<(), CleanupScheduleError> {
    let Ok(trigger) = CronTrigger::from_crontab(text, CronZone::Local) else {
        tracing::error!("Invalid maximum_spend_logs_cleanup_cron value: {text}");
        return Ok(());
    };
    scheduler.insert_job(cleanup_job_spec(Box::new(trigger), job))?;
    tracing::info!("Spend log cleanup scheduled with cron: {text}");
    Ok(())
}

fn startup_invalid_cron(value: &JsonValue) {
    tracing::error!(
        "Invalid maximum_spend_logs_cleanup_cron value: {}",
        value_text(value)
    );
}

fn startup_interval<R>(
    scheduler: &mut dyn CleanupScheduler,
    schedule: &CleanupSchedule,
    context: &ScheduleContext<'_>,
    job: JobFn,
    rng: &mut R,
) -> Result<(), CleanupScheduleError>
where
    R: Rng + ?Sized,
{
    let Some(ParsedInterval { text, seconds, .. }) = parsed_interval(schedule, context) else {
        return Ok(());
    };
    let offset = rng.random_range(0_u64..=60_u64);
    let shifted = seconds.saturating_add(offset);
    if interval_past_datetime_max(context.now, shifted) {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return Ok(());
    }
    scheduler.insert_job(cleanup_job_spec(
        Box::new(IntervalTrigger { seconds: shifted }),
        job,
    ))?;
    Ok(())
}
