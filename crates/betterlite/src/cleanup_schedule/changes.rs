use crate::job_scheduler::{JobFn, SchedulerState};

use super::SPEND_LOG_CLEANUP_JOB_ID;
use super::error::CleanupScheduleError;
use super::ports::{CleanupScheduler, GeneralSettingsView, cleanup_job_spec};
use super::schedule::{CleanupSchedule, cleanup_schedule_of, wants_cleanup_job};
use super::trigger::{ScheduleContext, build_trigger};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanupScheduleState {
    pub last_attempt: Option<CleanupSchedule>,
    pub reschedule_failed: bool,
}

pub struct ScheduleChange<'a> {
    pub previous: &'a CleanupSchedule,
    pub context: &'a ScheduleContext<'a>,
}

pub fn on_settings_changed(
    scheduler: Option<&mut dyn CleanupScheduler>,
    change: &ScheduleChange<'_>,
    current_settings: &dyn GeneralSettingsView,
    job: JobFn,
    state: &mut CleanupScheduleState,
) {
    if scheduler
        .as_ref()
        .is_some_and(|scheduler| scheduler.scheduler_state() == SchedulerState::Stopped)
    {
        return;
    }
    let current = cleanup_schedule_of(current_settings);
    let wanted = wants_cleanup_job(&current);
    let has_job = scheduler
        .as_ref()
        .is_some_and(|scheduler| scheduler.has_job(SPEND_LOG_CLEANUP_JOB_ID));
    let baseline = if has_job {
        state.last_attempt.as_ref().unwrap_or(change.previous)
    } else {
        change.previous
    };
    let retry_due = wanted
        && (!has_job || state.reschedule_failed)
        && state.last_attempt.as_ref() != Some(&current);
    if !(baseline != &current || retry_due || (has_job && !wanted)) {
        return;
    }
    match reschedule(scheduler, &current, change.context, job) {
        Ok(()) => {
            state.reschedule_failed = false;
            state.last_attempt = Some(current);
        }
        Err(error) => {
            state.reschedule_failed = true;
            tracing::error!(
                "Spend log cleanup could not be rescheduled, will retry on next sync: {error}"
            );
        }
    }
}

fn reschedule(
    scheduler: Option<&mut dyn CleanupScheduler>,
    current: &CleanupSchedule,
    context: &ScheduleContext<'_>,
    job: JobFn,
) -> Result<(), CleanupScheduleError> {
    let Some(scheduler) = scheduler else {
        return Ok(());
    };
    if !wants_cleanup_job(current) {
        if scheduler.has_job(SPEND_LOG_CLEANUP_JOB_ID) {
            scheduler.remove_existing_job(SPEND_LOG_CLEANUP_JOB_ID)?;
            tracing::info!("Removed existing spend log cleanup job");
        }
        return Ok(());
    }
    let Some(built) = build_trigger(current, context) else {
        return Ok(());
    };
    scheduler.insert_job(cleanup_job_spec(built.trigger, job))?;
    tracing::info!(
        "Spend log cleanup rescheduled with trigger: {}",
        built.description
    );
    Ok(())
}
