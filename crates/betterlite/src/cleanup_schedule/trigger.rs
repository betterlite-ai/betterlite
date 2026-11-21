use betterlite_config::sections::scheduled_job_stagger::Settings as StaggerSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, TimeDelta, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::job_scheduler::triggers::{CronTrigger, CronZone, IntervalTrigger};
use crate::scheduled_job_stagger::Trigger;
use crate::scheduled_job_stagger::apply::stagger_trigger;

use super::SPEND_LOG_CLEANUP_JOB_ID;
use super::ports::GeneralSettingsView;
use super::schedule::{CleanupSchedule, cleanup_schedule_of, is_truthy, value_text};

// APScheduler builds its start date as datetime.now() + interval, so a first fire past year 9999 raises OverflowError
const PYTHON_DATETIME_MAX_SECONDS: i64 = 253_402_300_799;

#[derive(Clone, Copy)]
pub struct ScheduleContext<'a> {
    pub stagger: &'a StaggerSettings,
    pub identity: &'a str,
    pub now: DateTime<Utc>,
    pub timezone: Option<&'a str>,
    pub database: &'a TimeZoneDatabase,
}

pub(super) struct BuiltTrigger {
    pub(super) trigger: Box<dyn Trigger>,
    pub(super) description: String,
}

pub(super) struct ParsedInterval<'a> {
    pub(super) text: &'a str,
    pub(super) seconds: u64,
    pub(super) period_seconds: Option<u32>,
}

#[must_use]
pub fn cleanup_trigger(
    settings: &dyn GeneralSettingsView,
    context: &ScheduleContext<'_>,
) -> Option<Box<dyn Trigger>> {
    build_trigger(&cleanup_schedule_of(settings), context).map(|built| built.trigger)
}

pub(super) fn build_trigger(
    schedule: &CleanupSchedule,
    context: &ScheduleContext<'_>,
) -> Option<BuiltTrigger> {
    let Some(cron) = schedule
        .cleanup_cron
        .as_ref()
        .filter(|value| is_truthy(value))
    else {
        return interval_built(schedule, context);
    };
    let Some(text) = cron.as_str() else {
        tracing::error!(
            "Invalid maximum_spend_logs_cleanup_cron value: {}",
            value_text(cron)
        );
        return None;
    };
    cron_built(text)
}

fn cron_built(text: &str) -> Option<BuiltTrigger> {
    match CronTrigger::from_crontab(text, CronZone::Local) {
        Ok(trigger) => Some(BuiltTrigger {
            trigger: Box::new(trigger),
            description: format!("cron {text}"),
        }),
        Err(_error) => {
            tracing::error!("Invalid maximum_spend_logs_cleanup_cron value: {text}");
            None
        }
    }
}

fn interval_built(
    schedule: &CleanupSchedule,
    context: &ScheduleContext<'_>,
) -> Option<BuiltTrigger> {
    let ParsedInterval {
        seconds,
        period_seconds,
        ..
    } = parsed_interval(schedule, context)?;
    Some(BuiltTrigger {
        trigger: stagger_trigger(
            SPEND_LOG_CLEANUP_JOB_ID,
            Box::new(IntervalTrigger { seconds }),
            period_seconds,
            context.stagger,
            context.identity,
        ),
        description: format!("interval of {seconds}s"),
    })
}

// APScheduler's IntervalTrigger raises OverflowError at construction for a first fire past datetime.max
#[must_use]
pub(super) fn interval_overflows_python_datetime(now: DateTime<Utc>, seconds: u64) -> bool {
    let Ok(seconds) = i64::try_from(seconds) else {
        return true;
    };
    let Some(delta) = TimeDelta::try_seconds(seconds) else {
        return true;
    };
    now.checked_add_signed(delta)
        .is_none_or(|fire| fire.timestamp() > PYTHON_DATETIME_MAX_SECONDS)
}

pub(super) fn parsed_interval<'a>(
    schedule: &'a CleanupSchedule,
    context: &ScheduleContext<'_>,
) -> Option<ParsedInterval<'a>> {
    let text = match schedule.cleanup_interval.as_ref() {
        None => "1d",
        Some(JsonValue::String(text)) => text.as_str(),
        Some(value) => {
            tracing::error!(
                "Invalid maximum_spend_logs_retention_interval value: {}",
                value_text(value)
            );
            return None;
        }
    };
    let Ok(parsed) = duration_in_seconds(text, context.now, context.timezone, context.database)
    else {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    };
    let Ok(seconds) = u64::try_from(parsed.seconds) else {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    };
    if interval_overflows_python_datetime(context.now, seconds) {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    }
    Some(ParsedInterval {
        text,
        seconds,
        period_seconds: u32::try_from(parsed.seconds).ok(),
    })
}
